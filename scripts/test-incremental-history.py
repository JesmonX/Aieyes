"""Protocol v2 regression fixtures; no SSH or personal histories are accessed."""
import contextlib
import io
import json
import os
import pathlib
import runpy
import sqlite3
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = pathlib.Path(__file__).with_name('remote_history.py')


def collect(path, provider='custom', cursors=None, route_cursor=None, precise=True,
            qoder_state_cursors=None, qoder_state_reset=False):
    output = io.StringIO()
    with contextlib.redirect_stdout(output):
        try:
            runpy.run_path(str(SCRIPT), init_globals={'REQUEST': dict(path=str(path), provider=provider, protocol=2, cursors=cursors or {}, routeCursor=route_cursor or {}, codexPreciseCounting=precise, qoderStateCursors=qoder_state_cursors or {}, qoderStateReset=qoder_state_reset)})
        except SystemExit as error:
            assert error.code == 0
    lines = output.getvalue().splitlines()
    assert all(len(line.encode()) + 1 <= 1024 * 1024 for line in lines)
    return [json.loads(line) for line in lines]


def cursors(frames):
    return {f['path']: f['cursor'] for f in frames if f['kind'] == 'batch' and f.get('cursor')}


def events(frames):
    return [e for f in frames if f['kind'] == 'batch' for e in f['events']]


def native_rows(frames):
    return [row for frame in frames for row in frame.get('nativeRows', [])]


class PiHistoryTests(unittest.TestCase):
    def test_codex_creator_identity_is_hashed_before_ssh_transport(self):
        import hashlib
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'rollout.jsonl'
            row = dict(type='session_meta', timestamp='2026-10-20T00:00:00Z', payload=dict(id='session', creator_user_id='PRIVATE_USER', creator_account_id='PRIVATE_ACCOUNT', base_instructions='PRIVATE_PROMPT'))
            path.write_text(json.dumps(row) + '\n')
            result = events(collect(path, provider='codex'))
            self.assertEqual(result[0]['payload']['creator_identity_key'], hashlib.sha256(b'PRIVATE_USER\0PRIVATE_ACCOUNT').hexdigest())
            self.assertNotIn('PRIVATE', json.dumps(result))

    def test_pi_family_projection_has_only_metering_and_appends_are_incremental(self):
        for provider in ('pi','omp'):
            with self.subTest(provider=provider),tempfile.TemporaryDirectory() as directory:
                path=pathlib.Path(directory)/'session.jsonl'
                fixture=SCRIPT.parents[1]/'crates/core/tests/fixtures/native-agents'/f'{provider}.jsonl'
                path.write_bytes(fixture.read_bytes())
                first=collect(path,provider);saved=cursors(first)
                self.assertNotIn('PRIVATE_',json.dumps(first))
                self.assertEqual(len([e for e in events(first) if e.get('message',{}).get('role')=='assistant']),1)
                repeat=collect(path,provider,saved)
                self.assertEqual(events(repeat),[])
                self.assertEqual(repeat[-1]['readBytes'],0)
                if provider=='omp':
                    path.write_bytes(path.read_bytes().replace(b'PRIVATE_TITLE',b'CHANGED_TITLE',1))
                    title=collect(path,provider,saved)
                    self.assertEqual(events(title),[])
                    self.assertEqual(title[-1]['readBytes'],0)
                    saved.update(cursors(title))
                row=dict(type='message',id='new-request',timestamp='2026-10-18T22:00:00Z',message=dict(role='assistant',model='auxiliary',usage=dict(input=10,output=2,cacheRead=3,cacheWrite=1),content='PRIVATE_OUTPUT'))
                with path.open('a') as stream:stream.write(json.dumps(row)+'\n')
                appended=collect(path,provider,saved)
                self.assertEqual(len(events(appended)),1)
                self.assertEqual(events(appended)[0]['message']['usage']['input'],10)
                self.assertNotIn('PRIVATE_',json.dumps(appended))


def record(number, model='fixture'):
    return json.dumps(dict(id=str(number), sessionId='s', model=model, timestamp=1800000000, tokens=dict(input=number))) + '\n'


class IncrementalHistory(unittest.TestCase):
    def test_codebuddy_credit_and_usage_sidecars_keep_identifiers_without_private_content(self):
        with tempfile.TemporaryDirectory() as directory:
            path=pathlib.Path(directory)/'session.jsonl'
            rows=[dict(type='message',role='user',sessionId='s',content='PRIVATE_PROMPT'),dict(type='model-usage',id='row',sessionId='s',timestamp=1800000000000,providerData=dict(messageId='m',conversationRequestId='turn',model='fixture',rawUsage=dict(input_tokens=100,output_tokens=20,credit=.5),extra='PRIVATE_USAGE')),dict(type='credit-usage',id='stage-row',sessionId='s',timestamp=1800000000000,providerData=dict(conversationRequestId='turn',source='team_member',stageId='team:row',credit=.5,extra='PRIVATE_STAGE'))]
            path.write_text(''.join(json.dumps(row)+'\n' for row in rows));first=collect(path,'codebuddy');data=events(first)
            self.assertNotIn('PRIVATE',json.dumps(first));self.assertEqual(data[0],dict(type='identity',sessionId='s'));self.assertEqual(data[1]['providerData']['rawUsage']['credit'],.5);self.assertEqual(data[2]['providerData']['stageId'],'team:row');self.assertEqual(events(collect(path,'codebuddy',cursors(first))),[])

    def test_cursor_private_store_warns_without_reading_native_transcripts_or_context_tokens(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory);db=root/'chats/workspace/session/store.db';db.parent.mkdir(parents=True);db.write_bytes(b'PRIVATE_DATABASE')
            transcript=root/'projects/p/agent-transcripts/t.jsonl';transcript.parent.mkdir(parents=True);transcript.write_text(record(999))
            export=root/'imports/usage.jsonl';export.parent.mkdir();export.write_text(record(20))
            first=collect(root,'cursor');self.assertTrue(first[-1]['partial']);self.assertEqual(first[-1]['issues'][0]['code'],'unsupportedNativeHistory');self.assertEqual([e['tokens']['input'] for e in events(first)],[20]);self.assertNotIn('PRIVATE',json.dumps(first))
            self.assertEqual(db.read_bytes(),b'PRIVATE_DATABASE');self.assertEqual(events(collect(root,'cursor',cursors(first))),[])
            self.assertFalse(collect(export,'cursor')[-1]['partial'])
            direct=collect(db,'cursor');self.assertTrue(direct[-1]['partial']);self.assertEqual(events(direct),[])
            standard=root/'standard/projects/usage.jsonl';standard.parent.mkdir(parents=True);standard.write_text(record(30))
            normal=collect(root/'standard','cursor');self.assertFalse(normal[-1]['partial']);self.assertEqual(events(normal)[0]['tokens']['input'],30)

    def test_qoder_request_credits_are_projected_without_body_or_extra_usage_fields(self):
        fixture=SCRIPT.parents[1]/'crates/core/tests/fixtures/native-agents/qoder.jsonl'
        row=json.loads(fixture.read_text().splitlines()[-1])
        row['message']['content']='PRIVATE_RESPONSE'
        row['message']['usage']['extra']='PRIVATE_USAGE'
        row['modelSource']='custom'
        row['isSidechain']=True
        with tempfile.TemporaryDirectory() as directory:
            path=pathlib.Path(directory)/'session.jsonl';path.write_text(json.dumps(row)+'\n')
            frames=collect(path,'qoder');projected=events(frames)[0]
            self.assertNotIn('PRIVATE_',json.dumps(frames))
            self.assertEqual(projected['modelSource'],'custom')
            self.assertIs(projected['isSidechain'],True)
            self.assertEqual(projected['message']['usage']['credits'],row['message']['usage']['credits'])
            self.assertIs(projected['message']['usage']['billable'],False)
            self.assertEqual(projected['message']['usage']['input_tokens'],0)
            self.assertEqual(events(collect(path,'qoder',cursors(frames))),[])
            normalized=dict(id='export',sessionId='s',timestamp=1800000000,model='qfmodel',tokens={},nativeCredits=dict(credits=.5,originalCredits=1,billable=False,extra='PRIVATE_EXPORT'))
            path.write_text(json.dumps(normalized)+'\n')
            export=collect(path,'qoder')
            self.assertNotIn('PRIVATE_',json.dumps(export))
            self.assertEqual(events(export)[0]['nativeCredits'],dict(credits=.5,originalCredits=1,billable=False))

    def test_qoder_state_sidecar_is_forwarded_without_paths_or_content_and_respects_cursor(self):
        import base64
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory);key=bytes(range(32));fake=root/'qodercli'
            fake.write_bytes(b'prefix Zjt=Buffer.from(['+b','.join(str(v).encode() for v in key)+b']) suffix')
            state=root/'projects/p/session/state.json';state.parent.mkdir(parents=True)
            payload=dict(sessionId='session',createdAt='2026-10-18T04:29:24.846Z',updatedAt='2026-10-18T04:29:24.851Z',workspaceDirectories=[str(root/'workspace')],items=dict(s0=dict(n=base64.b64encode(b'n'*12).decode(),p=base64.b64encode(b'PRIVATE_CIPHERTEXT').decode(),t=base64.b64encode(b't'*16).decode())))
            state.write_text(json.dumps(payload));transcript=root/'projects/p/session.jsonl';transcript.write_text(json.dumps(dict(type='assistant',cwd=str(root/'workspace')))+'\n')
            broken=root/'projects/p/broken/state.json';broken.parent.mkdir();broken.write_text('[]')
            with patch.dict(os.environ, {'QODER_CLI_PATH': str(fake)}):
                frames=collect(root,'qoder')
                self.assertNotIn('PRIVATE_',json.dumps(frames))
                states=[f for f in frames if f['kind']=='qoderState']
                self.assertTrue(frames[-1]['partial'])
                self.assertTrue(any(issue['code']=='qoderStateRead' for issue in frames[-1]['issues']))
                self.assertEqual(len(states),1)
                self.assertIn(base64.b64encode(key).decode(),states[0]['keys'])
                self.assertEqual(states[0]['states'][0]['sessionId'],'session')
                self.assertTrue(states[0]['states'][0]['projectHashes'])
                saved={states[0]['states'][0]['path']:states[0]['states'][0]['signature']}
                repeat=collect(root,'qoder',qoder_state_cursors=saved)
                self.assertEqual([f for f in repeat if f['kind']=='qoderState'],[])

    def test_opencode_real_messages_ignore_empty_future_table_and_private_parts(self):
        fixture=SCRIPT.parents[1]/'crates/core/tests/fixtures/native-agents/opencode-messages.json'
        for v2 in (False,True):
            with self.subTest(v2=v2),tempfile.TemporaryDirectory() as directory:
                root=pathlib.Path(directory);path=root/'opencode.db'
                with contextlib.closing(sqlite3.connect(path)) as db, db:
                    db.executescript('CREATE TABLE message(id TEXT,session_id TEXT,data TEXT);CREATE TABLE session_message(id TEXT,session_id TEXT,type TEXT,data TEXT);CREATE TABLE part(data TEXT);INSERT INTO part VALUES(\'PRIVATE_TOOL_RESULT\');')
                    for row in json.loads(fixture.read_text()):
                        data=row['data'];data['content']='PRIVATE_PROMPT';data['apiKey']='PRIVATE_KEY'
                        db.execute('INSERT INTO message VALUES(?,?,?)',(row['id'],row['session'],json.dumps(data)))
                        if v2:
                            data['model']=dict(id=data['modelID'],providerID=data['providerID'])
                            db.execute('INSERT INTO session_message VALUES(?,?,?,?)',(row['id'],row['session'],'assistant',json.dumps(data)))
                before=path.read_bytes();first=collect(root,'opencode')
                self.assertFalse(first[-1]['partial']);self.assertEqual(len(native_rows(first)),3)
                self.assertEqual(sum(e['data']['tokens']['input'] for e in native_rows(first)),8315)
                self.assertNotIn('PRIVATE_',json.dumps(first));self.assertEqual(before,path.read_bytes())
                self.assertEqual(collect(root,'opencode',cursors(first))[-1]['readBytes'],0)

    def test_native_clients_project_statistics_without_prompts_or_headers(self):
        fixtures=SCRIPT.parents[1]/'crates/core/tests/fixtures/native-agents'
        for provider,name,relative in [('kimi','kimi.jsonl','sessions/s/agents/main/wire.jsonl'),('codebuddy','codebuddy.jsonl','projects/p/s.jsonl'),('grok','grok-updates.jsonl','sessions/p/s/updates.jsonl')]:
            with self.subTest(provider=provider),tempfile.TemporaryDirectory() as directory:
                root=pathlib.Path(directory);path=root/relative;path.parent.mkdir(parents=True)
                rows=[json.loads(line) for line in (fixtures/name).read_text().splitlines()]
                for row in rows:
                    row['content']='PRIVATE_PROMPT';row['headers']={'Authorization':'PRIVATE_SECRET'}
                path.write_text(''.join(json.dumps(row)+'\n' for row in rows))
                first=collect(root,provider)
                self.assertEqual(first[-1]['kind'],'done')
                self.assertTrue(events(first))
                self.assertNotIn('PRIVATE_',json.dumps(first))
                second=collect(root,provider,cursors(first))
                self.assertEqual(events(second),[])

    def test_kimi_remote_fork_skips_inherited_request_counters(self):
        with tempfile.TemporaryDirectory() as directory:
            path=pathlib.Path(directory)/'wire.jsonl'
            usage=dict(type='usage.record',model='fixture',time=1792296701000,usage=dict(inputOther=10,output=3,inputCacheRead=0,inputCacheCreation=0))
            path.write_text(json.dumps(usage)+'\n'+json.dumps(dict(type='forked'))+'\n'+json.dumps(usage)+'\n')
            first=collect(path,'kimi')
            self.assertEqual(len(events(first)),1)
            self.assertEqual(events(collect(path,'kimi',cursors(first))),[])

    def test_zcode_native_sqlite_projection_is_readonly_and_incremental(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory);path=root/'cli/db/db.sqlite';path.parent.mkdir(parents=True)
            with contextlib.closing(sqlite3.connect(path)) as db, db:
                db.execute('CREATE TABLE model_usage(id TEXT,session_id TEXT,model_id TEXT,completed_at INTEGER,input_tokens INTEGER,output_tokens INTEGER,cache_read_input_tokens INTEGER,cache_creation_input_tokens INTEGER,reasoning_tokens INTEGER,provider_total_tokens INTEGER,computed_total_tokens INTEGER,status TEXT,raw_usage_json TEXT)')
                db.execute("INSERT INTO model_usage VALUES('request','session','qwen',1792297739892,100,25,60,10,5,125,125,'completed','PRIVATE_RESPONSE')")
            before=path.read_bytes();first=collect(root,'zcode')
            # Classification uses the same core parser as local SQLite reads.
            row=native_rows(first)[0]['data']
            self.assertEqual((row['input'],row['output'],row['read'],row['write'],row['reasoning'],row['providerTotal']),(100,25,60,10,5,125))
            self.assertNotIn('PRIVATE_RESPONSE',json.dumps(first))
            self.assertEqual(path.read_bytes(),before)
            self.assertEqual(native_rows(collect(root,'zcode',cursors(first))),[])

    def test_native_adapters_keep_standard_jsonl_exports_compatible(self):
        for provider in ['qoder','grok','kimi','codebuddy','pi','omp','opencode','zcode','deepseek-harness']:
            with self.subTest(provider=provider),tempfile.TemporaryDirectory() as directory:
                path=pathlib.Path(directory)/'export.jsonl';path.write_text(record(10))
                self.assertEqual(events(collect(path,provider))[0]['tokens']['input'],10)

    def test_disabled_precision_skips_parent_resolution_and_mode_change_replays(self):
        with tempfile.TemporaryDirectory() as root:
            home = pathlib.Path(root)
            child = dict(type='session_meta', payload=dict(id='child', history_base=dict(thread_id='missing', end_byte_offset=999999)))
            (home/'child.jsonl').write_text(json.dumps(child)+'\n'+json.dumps(dict(type='event_msg', timestamp=1800000000, payload=dict(type='token_count', info=dict(total_token_usage=dict(input_tokens=1000)))))+'\n')
            first = collect(home, 'codex', precise=False)
            self.assertFalse(first[-1]['partial'])
            self.assertEqual(first[-1]['forkHistoryIssues'], [])
            self.assertNotIn('inheritedHistory', events(first)[0])
            self.assertEqual(collect(home, 'codex', cursors(first), precise=False)[-1]['readBytes'], 0)
            exact = collect(home, 'codex', cursors(first), precise=True)
            self.assertTrue(next(f for f in exact if f['kind'] == 'batch')['reset'])
            self.assertFalse(exact[-1]['partial'])
            self.assertEqual(exact[-1]['forkHistoryIssues'][0]['code'], 'missingHistoryBase')
            self.assertEqual(exact[-1]['issues'], [])
            normal = collect(home, 'codex', cursors(exact), precise=False)
            self.assertTrue(next(f for f in normal if f['kind'] == 'batch')['reset'])
            self.assertNotIn('inheritedHistory', events(normal)[0])
            self.assertEqual(normal[-1]['forkHistoryIssues'], [])

    def test_renamed_parent_uses_metadata_and_recovers_existing_cursor(self):
        parent_id = '01a12393-1534-7750-8dbc-21bebb00678e'
        suffix_id = '01a13e39-8119-7f31-a38c-69d61a59ebfb'
        for newline in ('\n', '\r\n'):
            with self.subTest(newline=newline), tempfile.TemporaryDirectory() as root:
                home = pathlib.Path(root)
                def line(value): return (json.dumps(value, ensure_ascii=False) + newline).encode('utf-8')
                parent = line(dict(type='session_meta', payload=dict(id=parent_id)))
                parent += line(dict(type='response_item', payload=dict(text='PRIVATE_PARENT_TEXT 中文')))
                parent += line(dict(type='event_msg', timestamp=1800000000, payload=dict(type='token_count', info=dict(total_token_usage=dict(input_tokens=900)))))
                child = dict(type='session_meta', payload=dict(id='child', history_base=dict(thread_id=parent_id, end_byte_offset=len(parent))))
                (home/'child.jsonl').write_bytes(line(child))
                first = collect(home, 'codex')
                self.assertFalse(first[-1]['partial'])
                self.assertTrue(first[-1]['forkHistoryIssues'])
                (home/f'imported-{parent_id}_{suffix_id}.jsonl').write_bytes(parent)
                restored = collect(home, 'codex', cursors(first))
                self.assertFalse(restored[-1]['partial'])
                inherited = next(e['inheritedHistory'] for e in events(restored) if e.get('inheritedHistory'))
                self.assertEqual(inherited['tokens']['input'], 900)
                self.assertNotIn('PRIVATE_PARENT_TEXT', json.dumps(restored))
                self.assertEqual(collect(home, 'codex', cursors(restored))[-1]['readBytes'], 0)

    def test_parent_lookup_does_not_trust_a_stale_sqlite_pointer(self):
        with tempfile.TemporaryDirectory() as directory:
            home = pathlib.Path(directory)
            parent = json.dumps(dict(type='session_meta', payload=dict(id='parent'))) + '\n'
            parent += json.dumps(dict(type='event_msg', timestamp=1800000000, payload=dict(type='token_count', info=dict(total_token_usage=dict(input_tokens=900))))) + '\n'
            # history_base offsets count the physical UTF-8 bytes, without newline translation.
            (home/'parent-export.jsonl').write_bytes(parent.encode('utf-8'))
            wrong = home/'wrong.jsonl'
            wrong.write_text(json.dumps(dict(type='session_meta', payload=dict(id='different'))) + '\n')
            (home/'child.jsonl').write_text(json.dumps(dict(type='session_meta', payload=dict(id='child', history_base=dict(thread_id='parent', end_byte_offset=len(parent.encode()))))) + '\n')
            with contextlib.closing(sqlite3.connect(home/'state_5.sqlite')) as db, db:
                db.execute('CREATE TABLE threads(id TEXT,rollout_path TEXT)')
                db.execute('INSERT INTO threads VALUES(?,?)', ('parent', str(wrong)))
            first = collect(home, 'codex')
            self.assertEqual(first[-1]['forkHistoryIssues'], [])
            baseline = next(e['inheritedHistory'] for e in events(first) if e.get('inheritedHistory'))
            self.assertEqual(baseline['tokens']['input'], 900)

    def test_replaced_nonfork_file_does_not_retain_a_stale_inheritance_warning(self):
        with tempfile.TemporaryDirectory() as directory:
            home = pathlib.Path(directory)
            child = home/'child.jsonl'
            child.write_text(json.dumps(dict(type='session_meta', payload=dict(id='child', history_base=dict(thread_id='missing', end_byte_offset=123))))+'\n')
            first = collect(home, 'codex')
            self.assertEqual(first[-1]['forkHistoryIssues'][0]['reason'], 'parentNotFound')
            child.write_text(json.dumps(dict(type='session_meta', payload=dict(id='ordinary-session')))+'\n')
            replaced = collect(home, 'codex', cursors(first))
            self.assertEqual(replaced[-1]['forkHistoryIssues'], [])
            self.assertNotIn('inheritedHistory', cursors(replaced)[str(child)])
            self.assertEqual(collect(home, 'codex', cursors(replaced))[-1]['forkHistoryIssues'], [])

    def test_history_diagnostics_distinguish_ambiguous_corrupt_and_missing_records(self):
        thread = '01a12393-1534-7750-8dbc-21bebb00678e'
        for scenario, reason in [('missing', 'parentNotFound'), ('wrong-id', 'identityMismatch'),
                                 ('boundary', 'boundaryInvalid'), ('ambiguous', 'ambiguousHistory')]:
            with self.subTest(scenario=scenario), tempfile.TemporaryDirectory() as directory:
                home = pathlib.Path(directory)
                parent = json.dumps(dict(type='session_meta', payload=dict(id=thread))) + '\n'
                parent += json.dumps(dict(type='event_msg', timestamp=1800000000, payload=dict(type='token_count', info=dict(total_token_usage=dict(input_tokens=900))))) + '\n'
                if scenario != 'missing':
                    (home/f'rollout-2026-10-15T14-21-58-{thread}.jsonl').write_bytes((parent.replace(thread, 'different-parent') if scenario == 'wrong-id' else parent).encode('utf-8'))
                if scenario == 'ambiguous':
                    (home/'copy').mkdir()
                    (home/'copy'/f'rollout-2026-10-15T14-21-58-{thread}.jsonl').write_bytes(parent.replace('900', '800').encode('utf-8'))
                end = len(parent.encode()) - (1 if scenario == 'boundary' else 0)
                (home/'child.jsonl').write_text(json.dumps(dict(type='session_meta', payload=dict(id='child', history_base=dict(thread_id=thread, end_byte_offset=end)))) + '\n')
                first = collect(home, 'codex')
                self.assertFalse(first[-1]['partial'])
                self.assertEqual(first[-1]['issues'], [])
                diagnostic = first[-1]['forkHistoryIssues'][0]
                self.assertEqual(diagnostic['reason'], reason)
                self.assertEqual(diagnostic['parentRolloutId'], thread)
                repeated = collect(home, 'codex', cursors(first))
                self.assertEqual(repeated[-1]['forkHistoryIssues'][0]['reason'], reason)
                self.assertEqual(repeated[-1]['readBytes'], 0)

    def test_compressed_history_failure_reasons_and_bundled_ssh_helper(self):
        thread = '01a12393-1534-7750-8dbc-21bebb00678e'
        with tempfile.TemporaryDirectory() as directory:
            home = pathlib.Path(directory)
            rows = [dict(type='session_meta', payload=dict(id=thread)),
                    dict(type='response_item', payload=dict(text='PRIVATE_PARENT 中文')),
                    dict(type='event_msg', timestamp=1800000000, payload=dict(type='token_count', info=dict(total_token_usage=dict(input_tokens=900))))]
            def compressed(data):
                return subprocess.run(['node', '-e', "const z=require('node:zlib');process.stdout.write(z.zstdCompressSync(require('node:fs').readFileSync(0),{params:{[z.constants.ZSTD_c_checksumFlag]:1}}));"], input=data, capture_output=True, check=True).stdout
            data = [(json.dumps(row, ensure_ascii=False)+'\n').encode() for row in rows]
            packed = b''.join(compressed(row) for row in data)
            parent = home/f'rollout-2026-10-15T14-21-58-{thread}.jsonl.zst'
            parent.write_bytes(packed)
            child = dict(type='session_meta', payload=dict(id='child', history_base=dict(thread_id=thread, end_byte_offset=sum(map(len, data)))))
            (home/'child.jsonl').write_text(json.dumps(child)+'\n')
            def bundled(cursors=None):
                request = dict(protocol=2, path=str(home), provider='codex', codexPreciseCounting=True, cursors=cursors or {})
                scope = dict(REQUEST=request, HARNESS_SCRIPT=SCRIPT.with_name('harness_remote.py').read_text(encoding='utf-8'))
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    try:exec(compile(SCRIPT.read_text(encoding='utf-8'), '<wire-history>', 'exec'), scope)
                    except SystemExit as error:self.assertEqual(error.code, 0)
                self.assertNotIn('PRIVATE_PARENT', output.getvalue())
                return [json.loads(line) for line in output.getvalue().splitlines()]
            good = bundled()
            self.assertEqual(good[-1]['forkHistoryIssues'], [])
            baseline = events(good)[0]['inheritedHistory']
            self.assertEqual(baseline['tokens']['input'], 900)
            self.assertEqual(parent.read_bytes(), packed)
            with patch('shutil.which', return_value=None):
                unavailable = bundled(cursors(good))
                self.assertEqual(unavailable[-1]['forkHistoryIssues'][0]['reason'], 'decoderUnavailable')
                cached = bundled(cursors(unavailable))
                self.assertEqual(cached[-1]['forkHistoryIssues'][0]['reason'], 'decoderUnavailable')
                self.assertNotIn('尚未找到', cached[-1]['forkHistoryIssues'][0]['message'])
            recovered = bundled(cursors(unavailable))
            self.assertEqual(recovered[-1]['forkHistoryIssues'], [])
            self.assertEqual(events(recovered)[0]['inheritedHistory']['proof'], baseline['proof'])
            parent.write_bytes(packed[:-1])
            broken = bundled(cursors(recovered))
            self.assertEqual(broken[-1]['forkHistoryIssues'][0]['reason'], 'invalidCompressedHistory')
            self.assertFalse(broken[-1]['partial'])

    def test_fork_resolves_archived_multilevel_parent_and_rechecks_changed_proof(self):
        for newline in ('\n', '\r\n'):
            with self.subTest(newline=newline), tempfile.TemporaryDirectory() as root:
                home=pathlib.Path(root);sessions=home/'sessions/2026/10/13';archive=home/'archived_sessions'
                sessions.mkdir(parents=True);archive.mkdir()
                def line(value): return (json.dumps(value,ensure_ascii=False)+newline).encode('utf-8')
                def meta(identity, base=None): return line(dict(type='session_meta',payload=dict(id=identity,history_base=base) if base else dict(id=identity)))
                def count(total): return line(dict(type='event_msg',timestamp=1800000000,payload=dict(type='token_count',info=dict(total_token_usage=dict(input_tokens=total)))))
                parent=meta('parent')+line(dict(type='response_item',payload=dict(text='PRIVATE_PARENT_TEXT 中文')))+count(900)
                middle=meta('middle',dict(thread_id='parent',end_byte_offset=len(parent)))
                child=meta('child',dict(thread_id='middle',end_byte_offset=len(middle)))+count(1000)
                (archive/'parent.jsonl').write_bytes(parent+count(9999));(archive/'middle.jsonl').write_bytes(middle)
                (sessions/'child.jsonl').write_bytes(child)
                with contextlib.closing(sqlite3.connect(home/'state_5.sqlite')) as db:
                    db.execute('CREATE TABLE threads(id TEXT PRIMARY KEY,rollout_path TEXT)')
                    db.execute('INSERT INTO threads VALUES(?,?)',('parent',str(archive/'parent.jsonl')))
                    db.commit()
                first=collect(sessions,'codex')
                inherited=events(first)[0]['inheritedHistory']
                self.assertEqual(inherited['tokens']['input'],900)
                self.assertNotIn('PRIVATE_PARENT_TEXT',json.dumps(first))
                self.assertEqual(collect(sessions,'codex',cursors(first))[-1]['readBytes'],0)
                (archive/'parent.jsonl').write_bytes(parent.replace(b'900',b'800')+count(9999))
                changed=collect(sessions,'codex',cursors(first))
                self.assertEqual(events(changed)[0]['inheritedHistory']['tokens']['input'],800)
                self.assertTrue(changed[0]['reset'])
                self.assertNotEqual(events(changed)[0]['inheritedHistory']['proof'],inherited['proof'])

    def test_missing_fork_parent_recovers_without_a_stale_missing_issue(self):
        for newline in ('\n', '\r\n'):
            with self.subTest(newline=newline), tempfile.TemporaryDirectory() as root:
                home=pathlib.Path(root)
                parent=(json.dumps(dict(type='session_meta',payload=dict(id='parent')))+newline).encode('utf-8')
                child=dict(type='session_meta',payload=dict(id='child',history_base=dict(thread_id='parent',end_byte_offset=len(parent))))
                (home/'child.jsonl').write_bytes((json.dumps(child)+newline).encode('utf-8'))
                first=collect(home,'codex');self.assertFalse(first[-1]['partial'])
                self.assertEqual(first[-1]['forkHistoryIssues'][0]['code'],'missingHistoryBase')
                (home/'parent.jsonl').write_bytes(parent)
                restored=collect(home,'codex',cursors(first))
                self.assertFalse(restored[-1]['partial'])
                self.assertEqual(next(e for e in events(restored) if e.get('inheritedHistory'))['inheritedHistory']['tokens']['input'],0)

    def test_native_routes_are_incremental_and_never_export_private_request_fields(self):
        with tempfile.TemporaryDirectory() as root:
            home=pathlib.Path(root);sessions=home/'sessions/2026/10/13';sessions.mkdir(parents=True)
            with contextlib.closing(sqlite3.connect(home/'logs_2.sqlite')) as db:
                db.execute('CREATE TABLE logs(id INTEGER PRIMARY KEY,ts INTEGER,ts_nanos INTEGER,target TEXT,feedback_log_body TEXT,thread_id TEXT,process_uuid TEXT)')
                def insert(identity,body):
                    db.execute("INSERT INTO logs VALUES(?,1800000000,0,'codex_http_client::client',?,'session','process')",(identity,body));db.commit()
                insert(1,'url="https://secret:password@api.openai.com/v1/responses?api_key=PRIVATE" turn_id="turn" status=200 request_body="PROMPT"')
                insert(2,'url="https://api.openai.com/v1/models" status=200')
                first=collect(sessions,'codex');route=next(f for f in first if f['kind']=='routes')
                self.assertEqual(len(route['routes']),1)
                self.assertEqual(route['routes'][0]['category'],'api')
                self.assertEqual(route['routes'][0]['turnId'],'turn')
                for forbidden in ('secret','password','PRIVATE','PROMPT','api_key'): self.assertNotIn(forbidden,json.dumps(first))
                second=collect(sessions,'codex',route_cursor=route['cursor'])
                self.assertFalse(any(f['kind']=='routes' for f in second))
                insert(3,'url="https://chatgpt.com/backend-api/codex/responses" turn_id="next" status=200')
                third=collect(sessions,'codex',route_cursor=route['cursor'])
                new=next(f for f in third if f['kind']=='routes')
                self.assertEqual(len(new['routes']),1)
                self.assertEqual(new['routes'][0]['category'],'subscription')

    def test_append_partial_line_replacement_and_truncation(self):
        with tempfile.TemporaryDirectory() as root:
            path = pathlib.Path(root) / 'fixture.jsonl'
            path.write_text(record(1))
            first = collect(root)
            self.assertEqual(len(events(first)), 1)
            same = collect(root, cursors=cursors(first))
            self.assertEqual(events(same), [])
            self.assertEqual(same[-1]['readBytes'], 0)
            with path.open('a') as file:
                file.write(record(2) + record(3).rstrip('\n'))
            second = collect(root, cursors=cursors(first))
            self.assertEqual([e['id'] for e in events(second)], ['2'])
            self.assertFalse(next(f for f in second if f['kind'] == 'batch')['reset'])
            with path.open('a') as file:
                file.write('\n')
            third = collect(root, cursors=cursors(second))
            self.assertEqual([e['id'] for e in events(third)], ['3'])
            # Same path/size rewrite and inode replacement both restart parsing.
            path.write_text(record(4) + record(5) + record(6))
            rewrite = collect(root, cursors=cursors(third))
            self.assertTrue(rewrite[0]['reset'])
            self.assertEqual([e['id'] for e in events(rewrite)], ['4', '5', '6'])
            replacement = path.with_suffix('.replacement')
            replacement.write_text(record(7))
            os.replace(replacement, path)
            replaced = collect(root, cursors=cursors(rewrite))
            self.assertTrue(replaced[0]['reset'])
            path.write_text('')
            truncated = collect(root, cursors=cursors(replaced))
            self.assertEqual(truncated[0]['cursor']['offset'], 0)
            self.assertTrue(truncated[0]['reset'])

    def test_bounded_frames_and_restart_at_committed_checkpoint(self):
        with tempfile.TemporaryDirectory() as root:
            path = pathlib.Path(root) / 'fixture.jsonl'
            path.write_text(''.join(record(n, 'x' * 1800) for n in range(1700)))
            first = collect(root)
            batches = [f for f in first if f['kind'] == 'batch']
            self.assertGreater(len(batches), 3)
            # Simulate transport ending after one committed batch.
            resumed = collect(root, cursors=cursors(batches[:1]))
            combined = events(batches[:1]) + events(resumed)
            self.assertEqual([e['id'] for e in combined], [str(n) for n in range(1700)])
            self.assertEqual(collect(root, cursors=cursors(resumed))[-1]['readBytes'], 0)

    def test_nonstatistical_bytes_advance_and_private_text_never_leaves(self):
        with tempfile.TemporaryDirectory() as root:
            path = pathlib.Path(root) / 'fixture.jsonl'
            path.write_text((json.dumps({'type': 'response_item', 'payload': {'text': 'private-text' * 1000}}) + '\n') * 900)
            frames = collect(root, 'codex')
            self.assertEqual(events(frames), [])
            self.assertNotIn('private-text', json.dumps(frames))
            self.assertGreater(len(cursors(frames)), 0)
            self.assertGreater(len([f for f in frames if f['kind'] == 'batch']), 1)
            self.assertEqual(collect(root, 'codex', cursors(frames))[-1]['readBytes'], 0)

    def test_database_and_wal_fingerprints_and_failed_reads(self):
        with tempfile.TemporaryDirectory() as root:
            directory = pathlib.Path(root) / 'conversations'
            directory.mkdir()
            db = directory / 's.db'
            db.write_bytes(b'fixture')
            # Exercise incremental fingerprint/commit logic independently of the
            # protobuf parser, whose real SQLite fixtures live in core antigravity tests.
            with patch('sys.argv', ['collector', '/nonexistent', 'agy']), contextlib.redirect_stdout(io.StringIO()):
                module = runpy.run_path(str(SCRIPT))
            fn = module['_incremental']
            fn.__globals__['_agy_iter'] = lambda _: iter([{'event': json.loads(record(1))}])
            def scan(saved=None):
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    fn(dict(path=root, provider='antigravity', cursors=saved or {}))
                return [json.loads(line) for line in output.getvalue().splitlines()]
            first = scan()
            self.assertEqual(len(events(first)), 1)
            self.assertEqual(events(scan(cursors(first))), [])
            wal = directory / 's.db-wal'
            wal.write_bytes(b'new transaction')
            second = scan(cursors(first))
            self.assertEqual(len(events(second)), 1)
            self.assertEqual(events(scan(cursors(second))), [])
            wal.unlink()
            self.assertEqual(len(events(scan(cursors(second)))), 1)
            fn.__globals__['_agy_iter'] = lambda _: iter([{'issue': dict(code='invalidRecord', message='fixture')}])
            failed = scan()
            self.assertTrue(failed[-1]['partial'])
            self.assertEqual(cursors(failed), {})


if __name__ == '__main__':
    unittest.main()
