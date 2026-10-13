"""Responses gateway regression tests. All credentials and HTTP traffic are fixtures."""
import ast
import contextlib
import copy
import gzip
import http.client
import http.server
import importlib.util
import json
import pathlib
import subprocess
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

SCRIPT = pathlib.Path(__file__).with_name('remote_codex_api.py')
spec = importlib.util.spec_from_file_location('api_gateway', SCRIPT)
api = importlib.util.module_from_spec(spec)
spec.loader.exec_module(api)
TOMLI = {p.stem: p.read_text() for p in SCRIPT.parent.joinpath('vendor/tomli').glob('*.py')}
api.TOMLI = TOMLI


class Upstream(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def log_message(self, *_): pass
    def do_GET(self):
        self.send_response(200); self.send_header('Content-Length', '11'); self.end_headers(); self.wfile.write(b'{"data":[]}')
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
        if self.headers.get('Content-Encoding')=='gzip': body=gzip.decompress(body)
        value = json.loads(body)
        self.server.calls.append((dict(self.headers), value, self.path))
        model = value.get('model', 'fixture')
        usage = dict(input_tokens=100, output_tokens=20, input_tokens_details=dict(cached_tokens=10))
        response = dict(id='resp-'+model, object='response', status='completed', model=model, usage=usage)
        if model == 'missing': response.pop('usage')
        if model == 'compact': response['object'] = 'response.compaction'
        if model == 'invalid': response['usage']['input_tokens_details']['cached_tokens'] = -1
        if model == 'incomplete': response['status'] = 'incomplete'
        status = 429 if model == 'rejected' else 200
        if model in ('stream', 'cut'):
            events = [dict(type='response.created', response=dict(id=response['id'],model=model)),dict(type='response.completed', response=response)]
            data = b''.join(('data: '+json.dumps(event)+'\r\n\r\n').encode() for event in events)
            content_type = 'text/event-stream'
        else:
            data = json.dumps(response).encode(); content_type = 'application/json'
        self.send_response(status)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(data)+(20 if model=='cut' else 0)))
        self.send_header('x-request-id','fixture-request')
        self.send_header('Set-Cookie','must-not-be-forwarded')
        self.end_headers()
        for start in range(0,len(data),11): self.wfile.write(data[start:start+11]); self.wfile.flush()
        if model=='cut': self.close_connection=True


@contextlib.contextmanager
def server(handler):
    value = http.server.ThreadingHTTPServer(('127.0.0.1',0),handler)
    value.calls=[]
    worker=threading.Thread(target=value.serve_forever,kwargs={'poll_interval':.02},daemon=True);worker.start()
    try: yield value
    finally: value.shutdown();value.server_close();worker.join(3)


class APITests(unittest.TestCase):
    def setUp(self):
        self.temporary=tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.home=pathlib.Path(self.temporary.name).resolve()
        self.root=self.home/'.aieyes/api';api.private(self.root);api.private(self.root/'connections')
        self.connection=dict(id='fixture',revision='one',sourceId='source',name='Fixture',baseUrl='http://127.0.0.1:1/v1',model='fixture',effort='low',proxy={'mode':'direct'},prices=[])
        self.private=dict(connection=self.connection,apiKey='fixture-upstream-secret',localToken='fixture-local-token',proxyEnvironment={})
        self.cron=['5 6 * * * echo unrelated']
        self.writes=0
        def run(args,**kwargs):
            if args==['crontab','-l']: return subprocess.CompletedProcess(args,0,('\n'.join(self.cron)+'\n').encode(),b'')
            self.assertEqual(args,['crontab','-'])
            self.writes+=1;self.cron=kwargs['input'].decode().splitlines()
            return subprocess.CompletedProcess(args,0,b'',b'')
        self.mock_run=patch.object(api.subprocess,'run',side_effect=run)
        self.mock_run.start();self.addCleanup(self.mock_run.stop)

    def manage(self,action,**params):
        return api.manage(dict(action=action,home=str(self.home),sourceId='source',binary='/fixture/codex',params=params),SCRIPT.read_text(),TOMLI)

    def deploy(self,connection=None,**params):
        with patch.object(api,'ensure_started'):
            return self.manage('deploy',connection=connection or self.connection,apiKey='fixture-upstream-secret',**params)

    def post(self,port,model,authorization='fixture-local-token',compressed=False):
        client=http.client.HTTPConnection('127.0.0.1',port,timeout=3)
        body=json.dumps(dict(model=model,input='PRIVATE-PROMPT-FIXTURE')).encode()
        headers={'Authorization':'Bearer '+authorization,'Cookie':'private-cookie','x-codex-turn-metadata':json.dumps(dict(session_id='session',turn_id='turn-'+model)),'Content-Type':'application/json'}
        if compressed: body=gzip.compress(body);headers['Content-Encoding']='gzip'
        client.request('POST','/providers/fixture/v1/responses',body,headers)
        response=client.getresponse()
        try: payload=response.read()
        except http.client.IncompleteRead as error: payload=error.partial
        result=(response.status,dict(response.getheaders()),payload)
        client.close();return result

    def records(self,count):
        deadline=time.monotonic()+3
        while time.monotonic()<deadline:
            rows=api.ledger(self.root,-1)['records']
            if len(rows)==count and all(v['status']!='pending' for v in rows): return rows
            time.sleep(.01)
        self.fail('gateway did not finalize its ledger')

    def test_real_http_forwarding_sse_json_rejections_and_partial_usage(self):
        with server(Upstream) as upstream,server(api.Handler) as gateway:
            gateway.root=self.root
            self.connection['baseUrl']='http://127.0.0.1:%d/v1'%upstream.server_port
            api.atomic(self.root/'connections/fixture.json',self.private)
            denied=self.post(gateway.server_port,'fixture','wrong')
            self.assertEqual(denied[0],401);self.assertFalse(upstream.calls)
            for model in ('stream','fixture','missing','compact','invalid','rejected','incomplete','cut'):
                status,headers,_=self.post(gateway.server_port,model)
                self.assertEqual(status,429 if model=='rejected' else 200)
                self.assertEqual(headers['x-request-id'],'fixture-request')
                self.assertNotIn('Set-Cookie',headers)
            rows=self.records(8);by_model={v['model']:v for v in rows}
            self.assertEqual({k:v['status'] for k,v in by_model.items()},dict(stream='completed',fixture='completed',missing='incomplete',compact='completed',invalid='incomplete',rejected='rejected',incomplete='incomplete',cut='incomplete'))
            self.assertEqual(by_model['cut']['usage']['output_tokens'],20)
            self.assertEqual(by_model['stream']['sessionId'],'session')
            self.assertEqual(by_model['stream']['turnId'],'turn-stream')
            for headers,_,_ in upstream.calls:
                self.assertEqual(headers['Authorization'],'Bearer fixture-upstream-secret')
                self.assertNotIn('Cookie',headers)
            stored=json.dumps(rows)
            for forbidden in ('PRIVATE-PROMPT-FIXTURE','fixture-upstream-secret','fixture-local-token','private-cookie'):
                self.assertNotIn(forbidden,stored)

    def test_sse_multiline_created_id_and_usage_validation(self):
        capture=api.UsageCapture()
        capture.value(dict(type='response.created',response=dict(id='early',model='fixture')))
        self.assertEqual(capture.response['id'],'early');self.assertFalse(capture.complete)
        data=b'data: {"type":"response.completed",\r\ndata: "response":{"id":"r","usage":{"input_tokens":2,"output_tokens":1}}}\r\n\r\n'
        for start in range(0,len(data),3): capture.feed(data[start:start+3])
        self.assertTrue(capture.complete);self.assertEqual(capture.response['model'],'fixture')
        self.assertTrue(api.valid_usage(capture.response['usage']))
        for usage in ({'input_tokens':True,'output_tokens':1},{'input_tokens':1,'output_tokens':1,'input_tokens_details':{'cached_tokens':2}},{'input_tokens':1,'output_tokens':1,'input_tokens_details':'bad'}): self.assertFalse(api.valid_usage(usage))

    def test_compressed_request_keeps_encoding_and_uses_response_model(self):
        with server(Upstream) as upstream, server(api.Handler) as gateway:
            self.private['connection']['baseUrl']='http://127.0.0.1:%d/v1'%upstream.server_port
            api.atomic(self.root/'connections/fixture.json',self.private);gateway.root=self.root
            self.assertEqual(self.post(gateway.server_port,'stream',compressed=True)[0],200)
            record=self.records(1)[0]
            self.assertEqual((record['model'],record['status']),('stream','completed'))
            self.assertEqual(upstream.calls[0][0]['Content-Encoding'],'gzip')
            self.assertNotIn('PRIVATE-PROMPT-FIXTURE',json.dumps(record))

    def test_deploy_profile_cas_default_roundtrip_and_cleanup(self):
        (self.home/'auth.json').write_text('original shared credentials')
        (self.home/'config.toml').write_text('model="subscription-model"\n[projects.fixture]\ntrust_level="trusted"\n')
        mode=self.home.stat().st_mode
        result=self.deploy()
        self.assertIn('CODEX_HOME=',result['launchCommand']);self.assertTrue(result['resumeCommand'].endswith(' resume'))
        profile=self.home/(api.provider(self.connection)+'.config.toml')
        self.assertNotIn('fixture-upstream-secret',profile.read_text())
        self.assertEqual(self.home.stat().st_mode,mode)
        before=self.writes;api.register(self.root);self.assertEqual(self.writes,before)
        rev=api.revision(self.home)
        self.manage('default',id='fixture',revision='one',configRevision=rev)
        with self.assertRaisesRegex(ValueError,'配置已变化'): self.manage('default',id='subscription',configRevision=rev)
        updated=dict(self.connection,revision='two',model='new-api-model')
        self.deploy(updated,baseRevision='one',configRevision=api.revision(self.home))
        self.assertEqual(api.config(self.home)['model'],'new-api-model')
        self.assertNotIn(api.provider(self.connection),api.config(self.home)['model_providers'])
        with self.assertRaisesRegex(ValueError,'设置已变化'): self.manage('remove',id='fixture',revision='one')
        self.manage('default',id='subscription',configRevision=api.revision(self.home))
        self.assertEqual(api.config(self.home)['model'],'subscription-model')
        api.record(self.root,dict(id='pending',timestamp=1,status='pending'))
        self.manage('remove',id='fixture',revision='two')
        self.assertFalse(list(self.home.glob('aieyes_api_fixture_*.config.toml')))
        self.assertEqual(self.cron,['5 6 * * * echo unrelated'])
        self.assertFalse(api.has_connections(self.root))
        self.assertEqual(api.ledger(self.root,-1)['records'][0]['status'],'incomplete')
        self.assertEqual((self.home/'auth.json').read_text(),'original shared credentials')
        self.assertEqual(api.config(self.home)['projects']['fixture']['trust_level'],'trusted')
        self.assertFalse(any(name.startswith('aieyes_api_fixture_') for name in api.config(self.home)['model_providers']))

    def test_failed_deployment_restores_files_and_preserves_unrelated_cron(self):
        with patch.object(api,'ensure_started',side_effect=ValueError('not ready')):
            with self.assertRaisesRegex(ValueError,'not ready'): self.manage('deploy',connection=self.connection,apiKey='fixture-upstream-secret')
        self.assertFalse((self.root/'connections/fixture.json').exists())
        self.assertFalse((self.root/'runtime.json').exists())
        self.assertFalse((self.root/'gateway.py').exists())
        self.assertFalse(list(self.home.glob('*.config.toml')))
        self.assertEqual(self.cron,['5 6 * * * echo unrelated'])
        self.deploy()
        files={p:p.read_bytes() for p in (self.root/'gateway.py',self.root/'runtime.json',self.root/'connections/fixture.json')}
        changed=dict(self.connection,revision='two')
        with patch.object(api,'ensure_started',side_effect=ValueError('not ready')):
            with self.assertRaises(ValueError): self.manage('deploy',connection=changed,apiKey='new-key',baseRevision='one')
        self.assertEqual(files,{p:p.read_bytes() for p in files})
        self.assertFalse((self.home/(api.provider(changed)+'.config.toml')).exists())
        self.assertEqual(len(self.cron),2)

    def test_late_completion_cursor_and_service_restart(self):
        api.record(self.root,dict(id='slow',timestamp=1,status='pending'))
        first=api.ledger(self.root,-1)
        api.record(self.root,dict(id='recent',timestamp=100,status='completed'))
        second=api.ledger(self.root,first['cursor'])
        self.assertEqual(second['records'][0]['id'],'recent')
        api.interrupt_pending(self.root,'service-restarted')
        last=api.ledger(self.root,second['cursor'])
        self.assertEqual(last['records'][0]['id'],'slow');self.assertEqual(last['records'][0]['status'],'incomplete')
        self.assertGreater(last['cursor'],second['cursor'])

    def test_snapshot_never_replicates_provider_credentials(self):
        value=dict(model_provider='custom',model='alias',model_providers={'custom':{'base_url':'https://private.example/SECRET','experimental_bearer_token':'SECRET','auth':{'command':'/resolver/SECRET','args':['SECRET']}}})
        self.assertNotIn('SECRET',json.dumps(api.auth_snapshot(value)))
        self.assertEqual(api.subscription_backup(value),{'model_provider':'openai'})

    def test_python38_syntax_and_invalid_connection(self):
        ast.parse(SCRIPT.read_text(),feature_version=(3,8))
        with self.assertRaises(ValueError): self.deploy(dict(self.connection,revision='../unsafe'))
        with self.assertRaises(ValueError): self.deploy(dict(self.connection,baseUrl='https://key:secret@example.test/v1'))


if __name__=='__main__': unittest.main()
