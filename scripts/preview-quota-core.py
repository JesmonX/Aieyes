#!/usr/bin/env python3
"""Synthetic RPC fixture for native --render. No files, credentials or network reads."""
import json
import os
from datetime import date, timedelta
import sys
import time
stamp = int(os.environ.get('AIEYES_UI_STAMP', time.time()))
def source(provider):
    return dict(id=provider+'-source',name=provider+' · 测试来源',provider=provider,accountId=provider,path='/tmp/aieyes-preview-missing',enabled=True,quotaCommand='',quotaPreCommand='',codexBinary='codex',agyBinary='agy')
def window(key,name,used,minutes=10080,group=None):
    return dict(id=key,name=name,usedPercent=used,windowMinutes=minutes,resetsAt=stamp+minutes*60,groupName=group)
settings=dict(version=2,sources=[source('codex'),source('agy')],accounts=[dict(id=p,name=n,provider=p,quotaEnabled=True) for p,n in [('codex','个人订阅'),('agy','工作空间 · agy')]],hosts=[],proxy=dict(mode='system',url=''),refreshSeconds=300,serverRefreshSeconds=10,menuMetric='icon',githubRepository='',modelMappings={})
quotas=[dict(sourceId=p+'-source',accountId=p,provider=p,name=n,updatedAt=stamp,origin='live',windows=ws) for p,n,ws in [('codex','个人订阅',[window('codex:primary','5h',35,300),window('codex:secondary','7d',42)]),('agy','工作空间 · agy',[window(g+':'+str(m),g+' · '+('7d' if m==10080 else '5h'),v,m,g) for g,v in [('Claude Opus',28),('Claude Sonnet',65),('Gemini Pro',94)] for m in [300,10080]])]]
tokens=dict(input=30000,output=12000,cacheRead=28000,cacheWrite=0,reasoning=0)
usage=dict(key='',tokens=tokens,total=70000,cost=2.4,pricedTokens=70000,events=14)
price=dict(id='test-model',name='测试模型',input=0.00001,output=0.00003,cacheRead=0.000001,cacheWrite=0.00001,fetchedAt=stamp)
estimate=dict(id='preview',accountKey='codex:codex',windowId='codex:secondary',windowName='7d',sourceIds=['codex-source'],sourceNames=['本机 Codex'],status='pending',reason='额度已重置或异常回升；确认后开始新一段',startedAt=stamp-3600,checkpointAt=stamp-600,endedAt=None,consumedPercent=12,cost=2.4,totalTokens=70000,pricedTokens=70000,weeklyValue=20,calculationNote='手动采样估值 · 按本次模型组合估算',prices=[price])
estimate.update(valuationMode='fiveHour',windowId='codex:primary',windowName='5h',status='active',reason='',fiveHourValue=20,weeklyDirectValue=100,weeklyRatioValue=110,calculationNote='5h 采样估值 · 7d 同期样本较少',capacity=dict(ratio=5.5,samples=6,weeklyPercent=12,updatedAt=stamp),segments=[])
dashboard=dict(generatedAt=stamp,summary=usage,days=[],heatmap=[],models=[],trendDays=[],dayModels=[],pricingGaps=[],modelOptions=[],quotas=quotas,quotaOrder=['codex:codex','agy:agy'],quotaEstimates=[estimate],sources=[])
quotas[0].update(plan='plus',credits=dict(hasCredits=True,unlimited=False,balance='990.125'),creditsUpdatedAt=stamp)
dashboard['creditEstimates']=[dict(estimate,id='credit-preview',kind='credits',windowId='credits',windowName='Credits',status='active',reason='',weeklyValue=None,consumedCredits=10,valuePer500=120,valuePer1000=240)]
dashboard['creditEstimates'][0]['valuationMode']=''
task=dict(id='wake-preview',name='每日订阅唤醒',sourceId='codex-source',times=['08:00','13:30'],model='测试模型',effort='low',prompt='Hi. Reply only OK.',binary='')
wakeups=[dict(task=task,deployment=dict(task=task,deployedAt=stamp,enabled=True,state='deployed',timezone='CST +08:00',target='本机（模拟）',changed=False))]
# Optional 10-06 regression scenarios; the default preserves the quota fixture.
scenario = os.environ.get('AIEYES_UI_SCENARIO', '')
hosts = []
if scenario:
    settings['modelMappings'] = {'log-model-'+str(i):'provider/model-'+str(i) for i in range(30)}
    host = dict(id='preview-host', name='训练服务器', target='fixture.invalid', identityFile='', shell='/bin/bash', preCommand='', enabled=True, metrics=['cpu'], devices=[], details=[])
    settings['hosts'] = [host]
    settings['sources'][0]['hostId'] = host['id']
    hosts = [dict(id=host['id'], name=host['name'], sample=dict(timestamp=stamp, uptime=3600, load=[0.1], errors={}, cpu=[dict(id='cpu', utilization=42)]))]
    dashboard['quotaEstimates'] = []; dashboard['creditEstimates'] = []
    dashboard['trendDays'] = [dict(usage, key=(date(2026,10,1)+timedelta(days=i)).isoformat()) for i in range(7)]
    dashboard['heatmap'] = dashboard['trendDays']
    dashboard['models'] = [dict(usage, key='test-model')]
    dashboard['modelOptions'] = ['test-model']
    dashboard['dayModels'] = [dict(day=day['key'], model='test-model', usage=usage) for day in dashboard['trendDays']]
    if scenario == 'single':
        settings['sources'] = settings['sources'][:1]; settings['accounts'] = settings['accounts'][:1]; quotas = quotas[:1]
    elif scenario in ['multi', 'capacity']:
        quotas = [dict(quotas[0], accountId='account-'+str(i), sourceId='source-'+str(i), name='订阅账户 '+str(i+1)) for i in range(20 if scenario == 'capacity' else 5)]
        settings['accounts'] = [dict(id=q['accountId'], provider='codex', name=q['name'], quotaEnabled=True) for q in quotas]
        settings['sources'] = [dict(source('codex'), id=q['sourceId'], accountId=q['accountId'], name='日志 '+str(i+1)) for i,q in enumerate(quotas)]
        settings['hosts'] = [dict(host, id='host-'+str(i), name='训练服务器 '+str(i+1), metrics=['cpu','memory','gpu']) for i in range(5)]
        hosts = [dict(id=h['id'], name=h['name'], sample=dict(timestamp=stamp, uptime=3600, load=[0.1], errors={}, cpu=[dict(id='cpu',utilization=20+i*12)], memory=dict(total=16000000000,available=8000000000,cached=0,buffers=0,swapTotal=0,swapFree=0),gpu=[dict(id='0',utilization=40+i*10)])) for i,h in enumerate(settings['hosts'])]
        dashboard['quotaEstimates'] = [dict(estimate,accountKey='codex:account-0',sourceIds=['source-0'],sourceNames=['日志 1'])]
    elif scenario == 'long':
        long_name = '团队账户与训练服务器的长名称示例' * 4
        quotas[0]['name'] = long_name; settings['sources'][0]['name'] = long_name; settings['accounts'][0]['name'] = long_name
        usage.update(total=2400000000,cost=100000.25,pricedTokens=2400000000,tokens=dict(input=1500000000,output=300000000,cacheRead=500000000,cacheWrite=100000000,reasoning=0))
        dashboard['trendDays'] = [dict(usage,key=(date(2026,6,28)+timedelta(days=i)).isoformat()) for i in range(100)]
        dashboard['heatmap'] = [dict(usage,key=(date(2025,10,7)+timedelta(days=i)).isoformat()) for i in range(365)]
        dashboard['models'] = [dict(usage,key='provider/long-model-identifier-'+str(i),total=usage['total']/(i+1),cost=i+1) for i in range(12)]
        dashboard['modelOptions'] = [m['key'] for m in dashboard['models']]
        dashboard['dayModels'] = [dict(day=day['key'],model=m['key'],usage=m) for day in dashboard['trendDays'] for m in dashboard['models']]
    elif scenario == 'empty':
        settings['sources'] = []; settings['accounts'] = []; quotas = []
        dashboard['summary'] = dict(key='',tokens=dict(input=0,output=0,cacheRead=0,cacheWrite=0,reasoning=0),total=0,cost=0,pricedTokens=0,events=0)
        for key in ['trendDays','heatmap','models','dayModels','modelOptions']: dashboard[key] = []
    elif scenario == 'failure':
        for q in quotas: q['error'] = '连接失败：模拟账户凭据失效'
        hosts[0]['error'] = '连接失败：模拟 SSH 不可读取'
    dashboard['sources'] = [dict(src,status=dict(updatedAt=stamp,error='模拟读取失败' if scenario == 'failure' else None)) for src in settings['sources']]
    dashboard['quotas'] = quotas
    dashboard['quotaOrder'] = [q['provider']+':'+q['accountId'] for q in quotas]
# The browser and native renderer consume exactly the same synthetic data.
if '--fixture' in sys.argv:
    print(json.dumps(dict(settings=settings, dashboard=dashboard, prices=[price], hosts=hosts, wakeups=wakeups)))
    sys.exit(0)
for line in sys.stdin:
    r=json.loads(line)
    if r['method'] == 'dashboard' and r.get('params', {}).get('model') == 'ui-filter-failure':
        print(json.dumps(dict(jsonrpc='2.0', id=r['id'], error=dict(code=-32000, message='模拟筛选查询失败'))), flush=True)
        continue
    scan = [dict(id=src['id'], error='连接失败：模拟日志不可读取') for src in settings['sources']] if scenario == 'failure' else []
    network=dict(testedAt=stamp,mode='system',averageMs=73,status='ok',sites=[dict(url='https://api.github.com/rate_limit',latencyMs=60,error=None),dict(url='https://openrouter.ai',latencyMs=86,error=None)])
    result={'settings.get':settings,'dashboard':dashboard,'prices.list':[price],'sessions.list':[],'wakeups.list':wakeups,'sources.scan':scan,'quotas.refresh':quotas,'hosts.sample':hosts,'network.test':network,'network.status':network}.get(r['method'],{})
    print(json.dumps(dict(jsonrpc='2.0',id=r['id'],result=result)),flush=True)
