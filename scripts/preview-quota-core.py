#!/usr/bin/env python3
"""Synthetic RPC fixture for native --render. No files, credentials or network reads."""
import json
import sys
import time
stamp = int(time.time())
def source(provider):
    return dict(id=provider+'-source',name=provider+' · 测试来源',provider=provider,accountId=provider,path='/tmp/aieyes-preview-missing',enabled=True,quotaCommand='',quotaPreCommand='',codexBinary='codex',agyBinary='agy')
def window(key,name,used,minutes=10080,group=None):
    return dict(id=key,name=name,usedPercent=used,windowMinutes=minutes,resetsAt=stamp+86400,groupName=group)
settings=dict(version=2,sources=[source('codex'),source('agy')],accounts=[dict(id=p,name=n,provider=p,quotaEnabled=True) for p,n in [('codex','个人订阅'),('agy','工作空间 · agy')]],hosts=[],proxy=dict(mode='system',url=''),refreshSeconds=300,serverRefreshSeconds=10,menuMetric='icon',githubRepository='',modelMappings={})
quotas=[dict(sourceId=p+'-source',accountId=p,provider=p,name=n,updatedAt=stamp,origin='live',windows=ws) for p,n,ws in [('codex','个人订阅',[window('codex:primary','5h',35,300),window('codex:secondary','7d',42)]),('agy','工作空间 · agy',[window(g+':'+str(m),g+' · '+('7d' if m==10080 else '5h'),v,m,g) for g,v in [('Claude Opus',28),('Claude Sonnet',65),('Gemini Pro',94)] for m in [300,10080]])]]
tokens=dict(input=30000,output=12000,cacheRead=28000,cacheWrite=0,reasoning=0)
usage=dict(key='',tokens=tokens,total=70000,cost=2.4,pricedTokens=70000,events=14)
price=dict(id='test-model',name='测试模型',input=0.00001,output=0.00003,cacheRead=0.000001,cacheWrite=0.00001,fetchedAt=stamp)
estimate=dict(id='preview',accountKey='codex:codex',windowId='codex:secondary',windowName='7d',sourceIds=['codex-source'],sourceNames=['本机 Codex'],status='pending',reason='额度已重置或异常回升；确认后开始新一段',startedAt=stamp-3600,checkpointAt=stamp-600,endedAt=None,consumedPercent=12,cost=2.4,totalTokens=70000,pricedTokens=70000,weeklyValue=20,calculationNote='手动采样估值 · 按本次模型组合估算',prices=[price])
dashboard=dict(generatedAt=stamp,summary=usage,days=[],heatmap=[],models=[],trendDays=[],dayModels=[],pricingGaps=[],modelOptions=[],quotas=quotas,quotaOrder=['codex:codex','agy:agy'],quotaEstimates=[estimate],sources=[])
for line in sys.stdin:
    r=json.loads(line)
    result={'settings.get':settings,'dashboard':dashboard,'prices.list':[price],'sessions.list':[]}.get(r['method'],{})
    print(json.dumps(dict(jsonrpc='2.0',id=r['id'],result=result)),flush=True)
