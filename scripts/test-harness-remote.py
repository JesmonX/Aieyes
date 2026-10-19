"""Harness SSH projection uses local disposable files and Node; never contacts SSH."""
import contextlib
import importlib.util
import io
import json
import pathlib
import runpy
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT=pathlib.Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('harness',ROOT/'scripts/harness_remote.py')
h=importlib.util.module_from_spec(spec);spec.loader.exec_module(h)
spec=importlib.util.spec_from_file_location('sessions',ROOT/'scripts/session_remote.py')
s=importlib.util.module_from_spec(spec);spec.loader.exec_module(s)
FIXTURE=ROOT/'crates/core/tests/fixtures/native-agents/harness.jsonl'

def compressed(data):
    result=subprocess.run(['node','-e',"const z=require('node:zlib');process.stdout.write(z.zstdCompressSync(require('node:fs').readFileSync(0),{params:{[z.constants.ZSTD_c_checksumFlag]:1}}));"],input=data,capture_output=True,check=True)
    return result.stdout

def collect(path,cursors=None):
    request=dict(protocol=2,path=str(path),provider='deepseek-harness',cursors=cursors or {})
    output=io.StringIO()
    with contextlib.redirect_stdout(output):
        # Match the actual SSH loader: helper is bundled, no local __file__.
        scope=dict(__name__='__main__',REQUEST=request,HARNESS_SCRIPT=(ROOT/'scripts/harness_remote.py').read_text())
        try:exec(compile((ROOT/'scripts/remote_history.py').read_text(),'<wire-history>','exec'),scope)
        except SystemExit as error:assert error.code==0
    return [json.loads(line) for line in output.getvalue().splitlines()]

def cursors(frames):
    return {frame['path']:frame['cursor'] for frame in frames if frame['kind']=='batch' and frame.get('cursor')}

class HarnessProjection(unittest.TestCase):
    def test_v0_seed_length_and_generation_selection_preserve_request_coordinates(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory);legacy=root/'session.jsonl.zstd'
            rows=[json.loads(line) for line in FIXTURE.read_text().splitlines()]
            request=rows[3];request['seq']=2;request['content']='PRIVATE_BODY'
            inherited=json.loads(json.dumps(request));inherited['seq']=0;inherited['data']['usage']['inputTokens']=999
            old=[dict(type='session',version=0,id='session',seedLength=2),inherited,dict(type='permission/preset',seq=1),request]
            legacy.write_bytes(compressed(''.join(json.dumps(r)+'\n' for r in old).encode()))
            projected=list(h.project(legacy,'history'));self.assertEqual(projected[0]['harnessVersion'],0);self.assertFalse(projected[0]['isSeeded']);self.assertEqual(len(projected),2);self.assertEqual(projected[1]['data']['step'],1);self.assertEqual(projected[1]['seq'],2);self.assertNotIn('PRIVATE',json.dumps(projected))
            self.assertEqual(h.paths(root),[legacy])
            modern=root/'session.v4.jsonl';modern.write_text(FIXTURE.read_text())
            (root/'session.jsonl').write_text('{}\n')
            self.assertEqual(h.paths(root),[modern]);self.assertEqual(h.paths(legacy),[legacy])
            (root/'session.v4.jsonl.zstd').write_bytes(compressed(FIXTURE.read_bytes()))
            with self.assertRaisesRegex(ValueError,'conflicting'):h.paths(root)

    def test_concatenated_frames_match_plaintext_and_never_emit_message_content(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory);plain=root/'session.v4.jsonl';packed=root/'session.v4.jsonl.zstd'
            rows=[json.loads(line) for line in FIXTURE.read_text().splitlines()]
            for row in rows:row.update(content='PRIVATE_PROMPT',headers={'Authorization':'PRIVATE_KEY'})
            rows[3]['data']['message']['content']=[dict(type='text',text='PRIVATE_RESPONSE')]
            data=[(json.dumps(row)+'\n').encode() for row in rows];plain.write_bytes(b''.join(data));packed.write_bytes(b''.join(compressed(line) for line in data))
            before=packed.read_bytes();one=list(h.project(plain,'history'));two=list(h.project(packed,'history'))
            self.assertEqual(one,two);self.assertNotIn('PRIVATE_',json.dumps(two));self.assertEqual(packed.read_bytes(),before)
            self.assertEqual(two[1]['data']['usage']['inputTokens'],5621)
            self.assertEqual([r['kind'] for r in h.session(packed,{})['events']],['identity','start','thinking','complete'])

    def test_history_wire_is_cached_retried_and_retains_session_header_across_batches(self):
        with tempfile.TemporaryDirectory() as directory:
            path=pathlib.Path(directory)/'session.v4.jsonl.zstd'
            header=dict(type='session',version=4,id='session',isSeeded=True)
            usage=dict(type='assistant/message',seq=1,time=1792296856613,data=dict(turn=1,message=dict(source=dict(model='m'*400)),usage=dict(inputTokens=10,outputTokens=2)),content='PRIVATE_BODY')
            rows=[header,usage,dict(type='session/end-seed')]
            for i in range(2400):
                row=json.loads(json.dumps(usage));row['seq']=i+10;rows.append(row)
            path.write_bytes(compressed(''.join(json.dumps(row)+'\n' for row in rows).encode()))
            first=collect(path);self.assertFalse(first[-1]['partial']);self.assertNotIn('PRIVATE_',json.dumps(first))
            batches=[r for r in first if r['kind']=='batch'];self.assertGreater(len(batches),1)
            for index,batch in enumerate(batches):
                self.assertEqual(batch['events'][0]['id'],'session');self.assertEqual(batch['events'][0]['isSeeded'],index==0)
            again=collect(path,cursors(first));self.assertEqual(again[-1]['readBytes'],0);self.assertFalse(any(r['kind']=='batch' for r in again))
            extra=json.loads(json.dumps(usage));extra['seq']=9999
            appended=compressed((json.dumps(extra)+'\n').encode())
            with path.open('ab') as stream:stream.write(appended)
            changed=collect(path,cursors(first));self.assertFalse(changed[-1]['partial']);self.assertTrue(any(e.get('seq')==9999 for r in changed if r['kind']=='batch' for e in r['events']))
            self.assertEqual(changed[-1]['readBytes'],len(appended))

    def test_bad_frames_future_versions_conflicts_and_missing_runtime_are_explicit(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory);path=root/'session.v4.jsonl.zstd';good=compressed(FIXTURE.read_bytes());path.write_bytes(good)
            old=collect(path);saved=cursors(old)
            path.write_bytes(good[:-3]);failed=collect(path,saved);self.assertTrue(failed[-1]['partial']);self.assertEqual(failed[-1]['failedFiles'],1);self.assertFalse(any(r['kind']=='batch' for r in failed))
            corrupt=bytearray(good);corrupt[-1]^=1;path.write_bytes(corrupt)
            failed=collect(path,saved);self.assertTrue(failed[-1]['partial']);self.assertFalse(any(r['kind']=='batch' for r in failed))
            path.write_bytes(good)
            with patch.object(h.shutil,'which',return_value=None):
                with self.assertRaisesRegex(RuntimeError,'Node'):list(h.project(path,'history'))
            with patch.object(h,'fingerprint',side_effect=['before','after']):
                with self.assertRaisesRegex(RuntimeError,'changed during read'):list(h.project(path,'history'))
            with patch.object(h,'NODE',"process.stdout.write(JSON.stringify({type:'session',id:'partial'})+'\\n')"):
                with self.assertRaisesRegex(ValueError,'did not complete'):list(h.project(path,'history'))
            path.write_bytes(compressed(b'{"type":"session","version":5,"id":"s","content":"PRIVATE"}\n'))
            self.assertTrue(collect(path)[-1]['partial'])
            path.write_bytes(good);(root/'session.v4.jsonl').write_bytes(FIXTURE.read_bytes())
            with self.assertRaisesRegex(ValueError,'conflicting'):h.paths(root)
            self.assertEqual(h.paths(path),[path])
            (root/'session.v4.jsonl').unlink()
            (root/'session.v5.jsonl').write_text('{}\n')
            self.assertEqual(collect(root)[-1]['kind'],'error')

    def test_state_seed_boundary_and_failed_read_preserve_last_good_cursor(self):
        import time
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory);path=root/'sessions/project/session/session.v4.jsonl.zstd';path.parent.mkdir(parents=True)
            stamp=int(time.time()*1000)
            rows=[dict(type='session',version=4,id='s',isSeeded=True),dict(type='turn/end',time=stamp,data=dict(turn=1,reason=dict(kind='completed'))),dict(type='session/end-seed'),dict(type='turn/start',time=stamp,data=dict(turn=2)),dict(type='tool/call',time=stamp,data=dict(turn=2,id='call',name='bash',arguments='PRIVATE_ARGUMENT'))]
            path.write_bytes(compressed(''.join(json.dumps(row)+'\n' for row in rows).encode()))
            source=dict(id='harness',provider='deepseek-harness',path=str(root));first=s.collect(source,{})
            self.assertIsNone(first['error']);file=first['files'][0];self.assertEqual([e['kind'] for e in file['events']],['identity','start','tool']);self.assertNotIn('PRIVATE_',json.dumps(first))
            cached=s.collect(source,{str(path):file});self.assertEqual(cached['files'][0]['events'],[]);self.assertFalse(cached['files'][0]['reset'])
            limited=s.collect(source,{str(path):file},harness_deadline=0)
            self.assertIn('budget',limited['error']);self.assertEqual(limited['files'][0]['signature'],file['signature']);self.assertFalse(limited['files'][0]['reset'])
            added=compressed((json.dumps(dict(type='turn/end',time=stamp+1,data=dict(turn=2,reason=dict(kind='completed'))))+'\n').encode())
            with path.open('ab') as stream:stream.write(added)
            appended=s.collect(source,{str(path):file});self.assertIsNone(appended['error']);file=appended['files'][0]
            self.assertFalse(file['reset']);self.assertEqual(file['readBytes'],len(added));self.assertEqual([r['kind'] for r in file['events']],['identity','complete'])
            with path.open('ab') as stream:stream.write(b'BROKEN_PRIVATE')
            failed=s.collect(source,{str(path):file});self.assertTrue(failed['error']);self.assertNotIn('PRIVATE',json.dumps(failed));self.assertEqual(failed['files'][0]['signature'],file['signature']);self.assertFalse(failed['files'][0]['reset'])

if __name__=='__main__':unittest.main()
