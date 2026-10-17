import importlib.util,json,os,pathlib,sqlite3,subprocess,sys,tempfile,unittest
from unittest.mock import patch
ROOT=pathlib.Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('sessions',ROOT/'scripts/session_remote.py')
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);m.SCRIPT=(ROOT/'scripts/session_remote.py').read_text()
class ObserverTests(unittest.TestCase):
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
