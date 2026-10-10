#!/usr/bin/env python3
"""Compare full and incremental collectors on the same synthetic log; no SSH."""
import json
import pathlib
import subprocess
import sys
import tempfile
import time

ROOT=pathlib.Path(__file__).resolve().parents[1]
script=ROOT/'scripts/remote_history.py'
loader="import json,sys; p=json.load(sys.stdin); exec(compile(p['script'],'<fixture>','exec'),{'__name__':'__main__','REQUEST':p['request']})"
report={}
with tempfile.TemporaryDirectory(prefix='aieyes-remote-benchmark-') as directory:
    path=pathlib.Path(directory)/'session.jsonl'
    with path.open('w') as file:
        file.write(json.dumps(dict(type='session_meta',payload=dict(id='synthetic-session')))+'\n')
        file.write(json.dumps(dict(type='turn_context',payload=dict(model='fixture')))+'\n')
        for i in range(10000):
            file.write(json.dumps(dict(type='response_item',payload=dict(text='private synthetic text '*175)))+'\n')
            file.write(json.dumps(dict(type='event_msg',timestamp=1800000000+i,payload=dict(type='token_count',info=dict(total_token_usage=dict(input_tokens=i+1)))))+'\n')
    report['rawBytes']=path.stat().st_size
    runs=[]
    for _ in range(2):
        start=time.perf_counter();result=subprocess.run([sys.executable,str(script),directory,'codex'],capture_output=True,check=True)
        elapsed=(time.perf_counter()-start)*1000;data=json.loads(result.stdout)
        legacy=data['files'][0]['events'];runs.append(dict(ms=round(elapsed,2),replyBytes=len(result.stdout),events=len(legacy)))
    report['legacy']=runs
    cursors={};runs=[]
    for _ in range(2):
        payload=json.dumps(dict(script=script.read_text(),request=dict(protocol=2,path=directory,provider='codex',cursors=cursors))).encode()
        start=time.perf_counter();result=subprocess.run([sys.executable,'-c',loader],input=payload,capture_output=True,check=True)
        elapsed=(time.perf_counter()-start)*1000;frames=[json.loads(line) for line in result.stdout.splitlines()]
        projected=[e for f in frames if f['kind']=='batch' for e in f['events']]
        if not cursors:assert projected==legacy
        else:assert not projected and frames[-1]['readBytes']==0
        for frame in frames:
            if frame['kind']=='batch' and frame.get('cursor'):cursors[frame['path']]=frame['cursor']
        runs.append(dict(ms=round(elapsed,2),replyBytes=len(result.stdout),events=len(projected),readBytes=frames[-1]['readBytes']))
    report['incremental']=runs
output=ROOT/'.local/performance-implementation/remote-benchmark.json';output.parent.mkdir(parents=True,exist_ok=True);output.write_text(json.dumps(report,indent=2));print(json.dumps(report,indent=2))
