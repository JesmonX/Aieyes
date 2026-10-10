#!/usr/bin/env python3
"""Synthetic core load/soak. Uses only disposable data directories, never real SSH."""
import argparse
import json
import os
import pathlib
import sqlite3
import statistics
import subprocess
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]

class Core:
    def __init__(self, directory, binary):
        self.process = subprocess.Popen([str(binary)], env={**os.environ, 'AIEYES_DATA_DIR':str(directory)}, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.sequence = 0
    def call(self, method, params=None):
        self.sequence += 1
        start = time.perf_counter()
        self.process.stdin.write(json.dumps(dict(jsonrpc='2.0', id=self.sequence, method=method, params=params or {}))+'\n')
        self.process.stdin.flush()
        while True:
            line = self.process.stdout.readline()
            if not line: raise RuntimeError(self.process.stderr.read())
            response = json.loads(line)
            if response.get('id') != self.sequence: continue
            if 'error' in response: raise RuntimeError(response['error'])
            return response['result'], (time.perf_counter()-start)*1000, len(line.encode())
    def close(self):
        self.process.stdin.close()
        try: self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill(); self.process.wait()
        self.process.stdout.close(); self.process.stderr.close()


def seed(directory, count, binary):
    core = Core(directory, binary)
    core.call('hello')
    settings = core.call('settings.get')[0]
    hosts = 10 if count <= 100000 else 30
    settings.update(accounts=[dict(id=f'a{i}',provider='custom',name=f'Account {i}',quotaEnabled=False,archived=False) for i in range(hosts)], hosts=[], sources=[dict(id=f's{i}', provider='custom', accountId=f'a{i}', path='/synthetic', name=f'Synthetic {i}', enabled=True) for i in range(hosts)])
    settings['localMonitor']['enabled'] = False
    core.call('settings.save', settings); core.close()
    db = sqlite3.connect(directory/'aieyes.sqlite')
    now = int(time.time())
    db.execute('BEGIN')
    # Deterministic spread over 365 days, 30 accounts and 8 models.
    for start in range(0, count, 1000):
        records, memberships = [], []
        for i in range(start, min(start+1000,count)):
            source, account, model = f's{i%hosts}', f'a{i%hosts}', f'fixture-{i%8}'
            event = dict(id=f'e{i}', sourceId=source, accountId=account, provider='custom', model=model, timestamp=now-(i%365)*86400, sessionId=f'session-{i//30}', tokens=dict(input=100,output=20,cacheRead=10,cacheWrite=0,reasoning=0),attribution='fixture',billing=dict(category='unknown'),intervalStart=None,intervalEvidence='')
            records.append((event['id'],'custom',account,model,event['timestamp'],json.dumps(event,separators=(',',':')),0,0,None))
            memberships.append((event['id'],source))
        db.executemany('INSERT INTO events VALUES(?,?,?,?,?,?,?,?,?)',records)
        db.executemany('INSERT INTO event_sources VALUES(?,?)',memberships)
    db.commit();db.close()
    return settings


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--binary',type=pathlib.Path,default=ROOT/'target/release/aieyes-core')
    parser.add_argument('--output',type=pathlib.Path,default=ROOT/'.local/performance-implementation/synthetic.json')
    parser.add_argument('--soak-seconds',type=int,default=0)
    args=parser.parse_args();args.output.parent.mkdir(parents=True,exist_ok=True)
    report=dict(binary=str(args.binary),startedAt=time.strftime('%Y-%m-%dT%H:%M:%S%z'),datasets=[],soak=[])
    with tempfile.TemporaryDirectory(prefix='aieyes-performance-') as temporary:
        for count in (100000,1000000):
            directory=pathlib.Path(temporary)/str(count);directory.mkdir()
            seed(directory,count,args.binary)
            core=Core(directory,args.binary)
            try:
                core.call('hello')
                data=dict(events=count,hosts=10 if count==100000 else 30,queries={})
                for method,params in [('dashboard',{'days':1}),('dashboard',{'days':30}),('dashboard',{'days':365}),('dashboard.summary',{}),('dashboard',{'days':30,'sourceId':'s0'}),('dashboard',{'days':30,'accountId':'a0'})]:
                    runs=[core.call(method,params) for _ in range(5)]
                    key=method+json.dumps(params,sort_keys=True)
                    data['queries'][key]=dict(coldMs=round(runs[0][1],2),warmMedianMs=round(statistics.median(r[1] for r in runs[1:]),2),maxMs=round(max(r[1] for r in runs),2),replyBytes=runs[-1][2],total=runs[-1][0]['summary']['total'])
                report['datasets'].append(data);args.output.write_text(json.dumps(report,indent=2));print(json.dumps(data),flush=True)
                if count==1000000 and args.soak_seconds:
                    started=time.monotonic();iteration=0;report['soakStartedAt']=time.strftime('%Y-%m-%dT%H:%M:%S%z')
                    while time.monotonic()-started<args.soak_seconds:
                        result,duration,_=core.call('dashboard',{'days':[1,30,365][iteration%3]})
                        core.call('dashboard.summary')
                        # Periodic external writes invalidate only the touched day.
                        if iteration%15==0:
                            db=sqlite3.connect(directory/'aieyes.sqlite')
                            db.execute("UPDATE events SET cost=cost+0.000001 WHERE id='e0'");db.commit();db.close()
                        if iteration%15==0:
                            fields=subprocess.check_output(['ps','-p',str(core.process.pid),'-o','rss=,pcpu='],text=True).split()
                            children=subprocess.run(['pgrep','-P',str(core.process.pid)],capture_output=True,text=True).stdout.split()
                            db=sqlite3.connect(directory/'aieyes.sqlite');cache=db.execute('SELECT coalesce(sum(bytes),0) FROM usage_day_cache').fetchone()[0];db.close()
                            sample=dict(elapsed=round(time.monotonic()-started,1),iteration=iteration,rssKiB=int(fields[0]),cpuPercent=float(fields[1]),children=len(children),cacheBytes=cache,queryMs=round(duration,2))
                            report['soak'].append(sample);report['soakElapsedSeconds']=sample['elapsed'];args.output.write_text(json.dumps(report,indent=2));print(json.dumps(sample),flush=True)
                        iteration+=1;time.sleep(2)
                    report['soakElapsedSeconds']=round(time.monotonic()-started,1);report['soakComplete']=True;report['iterations']=iteration
                    args.output.write_text(json.dumps(report,indent=2))
            finally: core.close()
    report['finishedAt']=time.strftime('%Y-%m-%dT%H:%M:%S%z');args.output.write_text(json.dumps(report,indent=2))

if __name__=='__main__':main()
