"""Opt-in integration with an installed Codex CLI and local fixture endpoints only.

Run: python3 scripts/test-codex-api-cli.py /absolute/path/to/codex
No user auth files, API credentials, upstream services, or scheduler entries are used.
"""
import base64
import contextlib
import hashlib
import http.server
import importlib.util
import json
import os
import pathlib
import subprocess
import struct
import sys
import tempfile
import threading
import time

ROOT=pathlib.Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('api',ROOT/'remote_codex_api.py')
api=importlib.util.module_from_spec(spec);spec.loader.exec_module(api)
api.TOMLI={p.stem:p.read_text() for p in (ROOT/'vendor/tomli').glob('*.py')}


class Fixture(http.server.BaseHTTPRequestHandler):
    protocol_version='HTTP/1.1'
    def log_message(self,*_): pass
    def do_GET(self):
        if self.headers.get('Upgrade','').lower()=='websocket': return self.websocket()
        body=json.dumps(dict(models=[],data=[])).encode()
        self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
    def do_POST(self):
        body=self.rfile.read(int(self.headers.get('Content-Length',0)))
        try: request=json.loads(body)
        except ValueError: self.send_error(400);return
        if 'model' not in request:
            self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length','2');self.end_headers();self.wfile.write(b'{}');return
        events=self.events(request)
        data=b''.join(('data: '+json.dumps(event)+'\n\n').encode() for event in events)
        self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Content-Length',str(len(data)))
        if self.server.routes[-1]=='subscription':
            for key,value in {'x-codex-primary-used-percent':'1','x-codex-primary-window-minutes':'300','x-codex-primary-reset-at':str(int(time.time())+1000),'x-codex-plan-type':'plus'}.items(): self.send_header(key,value)
        self.end_headers();self.wfile.write(data)

    def events(self,request):
        route='api' if self.headers.get('Authorization')=='Bearer fixture-upstream-secret' else 'subscription'
        warmup=request.get('generate') is False
        if not warmup: self.server.routes.append(route)
        number=len(self.server.routes);rid='fixture-response-'+str(number)
        item=dict(id='fixture-message-'+str(number),type='message',role='assistant',status='completed',content=[dict(type='output_text',text='OK',annotations=[])])
        usage=dict(input_tokens=100,output_tokens=2,total_tokens=102,input_tokens_details=dict(cached_tokens=10),output_tokens_details=dict(reasoning_tokens=0))
        response=dict(id=rid,object='response',created_at=int(time.time()),status='completed',model=request['model'],output=[item],usage=usage)
        if warmup:
            response.update(output=[],usage=dict(input_tokens=0,output_tokens=0,total_tokens=0))
            return [dict(type='response.completed',response=response)]
        return [dict(type='response.created',response=dict(id=rid,object='response',status='in_progress',model=request['model'])),dict(type='response.output_item.done',output_index=0,item=item),dict(type='response.completed',response=response)]

    def websocket(self):
        key=self.headers['Sec-WebSocket-Key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11'
        self.send_response(101);self.send_header('Upgrade','websocket');self.send_header('Connection','Upgrade');self.send_header('Sec-WebSocket-Accept',base64.b64encode(hashlib.sha1(key.encode()).digest()).decode());self.end_headers()
        while True:
            header=self.rfile.read(2)
            if len(header)!=2 or header[0]&15==8: break
            length=header[1]&127
            if length==126: length=struct.unpack('!H',self.rfile.read(2))[0]
            elif length==127: length=struct.unpack('!Q',self.rfile.read(8))[0]
            mask=self.rfile.read(4) if header[1]&128 else b''
            payload=self.rfile.read(length)
            if mask: payload=bytes(v^mask[i%4] for i,v in enumerate(payload))
            if header[0]&15!=1: continue
            request=json.loads(payload)
            for event in self.events(request.get('response',request)):
                data=json.dumps(event).encode()
                frame=bytes([129,len(data)]) if len(data)<126 else bytes([129,126])+struct.pack('!H',len(data))
                self.wfile.write(frame+data);self.wfile.flush()
        self.close_connection=True


@contextlib.contextmanager
def serve(handler):
    server=http.server.ThreadingHTTPServer(('127.0.0.1',0),handler);server.routes=[]
    worker=threading.Thread(target=server.serve_forever,kwargs={'poll_interval':.02},daemon=True);worker.start()
    try: yield server
    finally: server.shutdown();server.server_close();worker.join(5)


def run(binary):
    with tempfile.TemporaryDirectory(prefix='aieyes-api-cli-') as temporary,serve(Fixture) as upstream:
        home=pathlib.Path(temporary).resolve();root=home/'.aieyes/api'
        endpoint='http://127.0.0.1:%d'%upstream.server_port
        c=dict(id='fixture',revision='one',sourceId='cli',name='Fixture API',baseUrl=endpoint+'/v1')
        claims={'exp':4102444800,'email':'fixture@example.invalid','https://api.openai.com/auth':{'chatgpt_user_id':'fixture-user','chatgpt_account_id':'fixture-workspace','chatgpt_plan_type':'plus'}}
        token='e30.'+base64.urlsafe_b64encode(json.dumps(claims).encode()).decode().rstrip('=')+'.fixture'
        auth=dict(auth_mode='chatgpt',tokens=dict(access_token=token,id_token=token,refresh_token='fixture-refresh-token',account_id='fixture-workspace'))
        api.atomic(home/'auth.json',auth)
        values=dict(model='gpt-6-luna',model_reasoning_effort='low',openai_base_url=endpoint+'/v1',chatgpt_base_url=endpoint,analytics={'enabled':False},features={'shell_tool':False,'apps':False,'enable_request_compression':False})
        api.atomic(home/'config.toml',api.toml_text(values),raw=True)
        original_auth=(home/'auth.json').read_bytes()
        original_config=(home/'config.toml').read_bytes()
        request=dict(home=str(home),sourceId='cli',binary=binary)
        api.direct_manage(dict(request,action='deploy',params=dict(connection=c,apiKey='fixture-upstream-secret',baseRevision='')),'',{})
        assert (home/'config.toml').read_bytes()==original_config
        assert not (root/'runtime.json').exists() and not (root/'gateway.py').exists()
        env={k:v for k,v in os.environ.items() if k in ('PATH','HOME','TMPDIR','LANG','LC_ALL','SYSTEMROOT','WINDIR')}
        env['CODEX_HOME']=str(home)
        def cli(session=None):
            args=[binary,'--no-daemon']
            args+=['exec','--skip-git-repo-check','--json']
            if session: args+=['resume',session]
            args+=['Reply only OK. Do not use tools.']
            result=subprocess.run(args,cwd=home,env=env,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=45)
            rows=[]
            for line in result.stdout.decode().splitlines():
                with contextlib.suppress(ValueError): rows.append(json.loads(line))
            if result.returncode or not any(row.get('type')=='turn.completed' for row in rows):
                detail=result.stderr.decode()+result.stdout.decode()
                for secret in (token,'fixture-upstream-secret','fixture-refresh-token'): detail=detail.replace(secret,'[fixture credential]')
                raise AssertionError('Codex fixture call failed:\n'+detail[-6000:])
            return next(row['thread_id'] for row in rows if row.get('type')=='thread.started')
        # The coordinator closes reviewed instances before this native selection.
        # Each fixture invocation has already exited, so there are no processes to close.
        session=cli()
        for target in [dict(kind='api',connectionId=c['id'],revision=c['revision']),dict(kind='account'),dict(kind='api',connectionId=c['id'],revision=c['revision'])]:
            prepared=api.direct_prepare(home,target)
            api.direct_select(home,prepared)
            assert cli(session)==session
            assert (home/'auth.json').read_bytes()==original_auth
            current=api.config(home)
            for key,value in values.items(): assert current[key]==value
        assert upstream.routes==['subscription','api','subscription','api'],upstream.routes
        assert (home/'auth.json').read_bytes()==original_auth
        assert not (root/'meter.sqlite').exists() and not (root/'runtime.json').exists()
        print(json.dumps(dict(cli=pathlib.Path(binary).name,turns=len(upstream.routes),routes=upstream.routes,sameConversationResumed=True,sharedAuthUnchanged=True,noGateway=True)))


if __name__=='__main__':
    if len(sys.argv)!=2: raise SystemExit(__doc__)
    run(str(pathlib.Path(sys.argv[1]).resolve()))
