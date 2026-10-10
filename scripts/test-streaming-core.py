#!/usr/bin/env python3
"""End-to-end core -> fake SSH -> Python -> transactional incremental import."""
import concurrent.futures
import json
import os
import pathlib
import sqlite3
import subprocess
import tempfile
import signal
import time
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = pathlib.Path(os.environ.get('AIEYES_TEST_CORE', ROOT/'target/debug/aieyes-core'))

class Core:
    def __init__(self, data, bin_directory):
        self.child = subprocess.Popen([str(BINARY)],env={**os.environ,'AIEYES_DATA_DIR':str(data),'PATH':str(bin_directory)+os.pathsep+os.environ['PATH']},stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        self.sequence=0
    def call(self, method, params=None):
        self.sequence+=1
        self.child.stdin.write(json.dumps(dict(id=self.sequence,method=method,params=params or {}))+'\n');self.child.stdin.flush()
        while True:
            line=self.child.stdout.readline()
            if not line: raise RuntimeError(self.child.stderr.read())
            response=json.loads(line)
            if response.get('id')!=self.sequence:continue
            if 'error' in response:raise RuntimeError(response['error'])
            return response['result']
    def close(self):
        self.child.stdin.close()
        try:self.child.wait(timeout=5)
        except subprocess.TimeoutExpired:self.child.kill();self.child.wait()
        self.child.stdout.close();self.child.stderr.close()


class StreamingCore(unittest.TestCase):
    def test_corrupt_frame_after_stream_start_does_not_report_success(self):
        with tempfile.TemporaryDirectory(prefix='aieyes-corrupt-stream-') as root:
            root = pathlib.Path(root)
            directory = root/'bin'; directory.mkdir()
            ssh = directory/'ssh'
            batch = dict(protocol=2, kind='batch', path='/fixture', reset=True,
                         events=[], cursor=dict(offset=0))
            done = dict(protocol=2, kind='done', files=1, failedFiles=0)
            output = '\n'.join([json.dumps(batch), 'corrupt frame', json.dumps(done)])+'\n'
            ssh.write_text('#!/usr/bin/env python3\nimport sys\nsys.stdin.read()\nsys.stdout.write('+repr(output)+')\n')
            ssh.chmod(0o700)
            core = Core(root/'data', directory)
            try:
                settings = core.call('settings.get')
                settings.update(accounts=[], sources=[dict(id='s', provider='codex',
                    accountId='', path='/fixture', enabled=True, hostId='fixture')],
                    hosts=[dict(id='fixture', target='synthetic.invalid', enabled=True)])
                settings['localMonitor']['enabled'] = False
                core.call('settings.save', settings)
                result = core.call('sources.scan')[0]
                self.assertIn('error', result)
                self.assertIn('损坏', result['error'])
            finally:
                core.close()

    def test_cursor_atomicity_replay_and_parallel_source_lock(self):
        with tempfile.TemporaryDirectory(prefix='aieyes-streaming-') as root:
            root=pathlib.Path(root);bin_directory=root/'bin';bin_directory.mkdir();data=root/'data';history=root/'history';history.mkdir()
            ssh=bin_directory/'ssh'
            ssh.write_text("""#!/usr/bin/env python3
import subprocess, sys
print('fixture login banner without newline', end='', flush=True)
raise SystemExit(subprocess.call(sys.argv[-1], shell=True))
""");ssh.chmod(0o700)
            log=history/'fixture.jsonl'
            def token(n):return dict(type='event_msg',timestamp=1800000000,payload=dict(type='token_count',info=dict(total_token_usage=dict(input_tokens=n))))
            rows=[dict(type='session_meta',payload=dict(id='fixture')),dict(type='turn_context',payload=dict(model='fixture')),token(100)]
            log.write_text(''.join(json.dumps(row)+'\n' for row in rows))
            core=Core(data,bin_directory);other=None
            try:
                settings=core.call('settings.get');settings.update(accounts=[],sources=[dict(id='s',provider='codex',accountId='',path=str(history),name='Fixture',enabled=True,hostId='fixture')],hosts=[dict(id='fixture',target='synthetic.invalid',name='Fixture',enabled=True,shell='/bin/sh')]);settings['localMonitor']['enabled']=False
                core.call('settings.save',settings)
                first=core.call('sources.scan')[0];self.assertNotIn('error',first);self.assertEqual(first['result']['newEvents'],1)
                same=core.call('sources.scan')[0];self.assertEqual(same['result']['readBytes'],0);self.assertEqual(same['result']['newEvents'],0)
                with log.open('a') as f:f.write(json.dumps(token(150))+'\n')
                other=Core(data,bin_directory)
                with concurrent.futures.ThreadPoolExecutor(2) as executor:
                    results=list(executor.map(lambda c:c.call('sources.scan'),[core,other]))
                self.assertTrue(all('error' not in result[0] for result in results))
                db=sqlite3.connect(data/'aieyes.sqlite')
                total=sum(json.loads(r[0])['tokens']['input'] for r in db.execute('SELECT payload FROM events'))
                self.assertEqual(total,150)
                cursor_before=db.execute('SELECT cursor FROM remote_cursors').fetchone()[0]
                db.execute("CREATE TRIGGER reject_remote_cursor BEFORE UPDATE ON remote_cursors BEGIN SELECT RAISE(ABORT,'fixture rejection'); END;");db.commit()
                with log.open('a') as f:f.write(json.dumps(token(200))+'\n')
                failed=core.call('sources.scan')[0];self.assertIn('error',failed)
                self.assertEqual(db.execute('SELECT cursor FROM remote_cursors').fetchone()[0],cursor_before)
                self.assertEqual(sum(json.loads(r[0])['tokens']['input'] for r in db.execute('SELECT payload FROM events')),150)
                db.execute('DROP TRIGGER reject_remote_cursor');db.commit()
                resumed=core.call('sources.scan')[0];self.assertNotIn('error',resumed)
                self.assertEqual(sum(json.loads(r[0])['tokens']['input'] for r in db.execute('SELECT payload FROM events')),200)
                self.assertEqual(json.loads(db.execute('SELECT cursor FROM remote_cursors').fetchone()[0])['offset'],log.stat().st_size)
                db.close()
                settings['hosts'][0]['enabled']=False
                core.call('settings.save',settings)
                self.assertEqual(core.call('sources.scan'),[],'Automatic scans skip paused hosts')
                self.assertIn('error',core.call('sources.scan',{'sourceId':'s'})[0],'Explicit boundary reads cannot claim a paused source succeeded')
            finally:
                core.close()
                if other:other.close()

    @unittest.skipUnless(os.name == 'posix', 'Unix process-group shutdown')
    def test_stopping_core_kills_ongoing_ssh_and_its_descendant(self):
        with tempfile.TemporaryDirectory(prefix='aieyes-shutdown-') as root:
            root=pathlib.Path(root);directory=root/'bin';directory.mkdir();data=root/'data'
            ssh=directory/'ssh'
            ssh.write_text("#!/usr/bin/env python3\nimport subprocess,os,time,pathlib\nchild=subprocess.Popen(['sleep','60'])\npathlib.Path(__file__).with_name('pids').write_text(str(os.getpid())+' '+str(child.pid))\ntime.sleep(60)\n")
            ssh.chmod(0o700)
            core=Core(data,directory);pids=[]
            try:
                settings=core.call('settings.get');settings.update(accounts=[],sources=[],hosts=[dict(id='fixture',name='Fixture',target='synthetic.invalid',enabled=True,metrics=['cpu'])]);settings['localMonitor']['enabled']=False
                core.call('settings.save',settings)
                core.child.stdin.write(json.dumps(dict(id=99,method='hosts.sample',params={'stream':True}))+'\n');core.child.stdin.flush()
                deadline=time.monotonic()+5
                while not (directory/'pids').exists() and time.monotonic()<deadline:time.sleep(0.01)
                pids=list(map(int,(directory/'pids').read_text().split()))
                core.child.terminate();core.child.wait(timeout=3)
                for pid in pids:
                    deadline=time.monotonic()+3
                    while time.monotonic()<deadline:
                        status=subprocess.run(['ps','-p',str(pid),'-o','stat='],capture_output=True,text=True).stdout.strip()
                        if not status or status.startswith('Z'):break
                        time.sleep(0.02)
                    else:self.fail('query subprocess remained alive after core exit')
            finally:
                core.close()
                for pid in pids:
                    try:os.kill(pid,signal.SIGKILL)
                    except ProcessLookupError:pass

if __name__=='__main__':unittest.main()
