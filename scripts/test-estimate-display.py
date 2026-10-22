#!/usr/bin/env python3
"""Compare full/display histories with the production Web and Swift card selectors."""
import importlib.util
import json
import sys
from pathlib import Path
import sqlite3
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('performance', ROOT/'scripts/benchmark-performance.py')
performance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(performance)

with tempfile.TemporaryDirectory(prefix='aieyes-display-golden-') as temporary:
    root = Path(temporary)
    core = performance.Core(root, ROOT/'target/debug/aieyes-core')
    core.call('hello')
    db = sqlite3.connect(root/'aieyes.sqlite')
    groups = [('codex:a','quota',''),('antigravity:b','quota','Gemini Models'),('antigravity:b','quota','Claude and GPT Models'),('codex:a','credits','')]
    for account, kind, group in groups:
        prefix = account+kind+group
        for i in range(120):
            row = dict(id=prefix+str(i), accountKey=account, groupId=group or None, status='pending' if i==119 and (kind=='credits' or group=='Gemini Models') else 'active' if i==118 and account=='codex:a' and kind=='quota' else 'completed', startedAt=1000+i//2, repairedAt=i%5, checkpointAt=2000+i, valuationMode='fiveHour', quotaPlan='plus', checkpointWindows=[dict(id='five',name='5h',windowMinutes=300,groupId=group),dict(id='week',name='7d',windowMinutes=10080,groupId=group)], weeklyValue=float(i) if i%3 else None, fiveHourValue=float(i) if i%5 else None, weeklyRatioValue=float(i) if i%7 else None, valuePer1000=float(i) if i%3 else None, prices=[dict(id='frozen',name='Frozen model',input=1e-6,output=2e-6,fetchedAt=0)], originalEstimateId=prefix+str(i-1) if i and i%13==0 else None)
            db.execute('INSERT INTO estimates VALUES(?,?,?,?,?)',(kind,row['id'],account,row['status'],json.dumps(row)))
    db.commit()
    db.close()
    try:
        full = core.call('dashboard.summary',{'estimateHistory':'full'})[0]
        display = core.call('dashboard.summary',{'estimateHistory':'display'})[0]
    finally:
        core.close()
    cases=[]
    for account, kind, group in groups:
        field='creditEstimates' if kind=='credits' else 'quotaEstimates'
        select=lambda rows:[r for r in rows if r['accountKey']==account and (r.get('groupId') or '')==group]
        quota=dict(sourceId='fixture',origin='live',provider=account.split(':')[0],accountId=account.split(':')[1],name='Fixture',updatedAt=2000,windows=[dict(id='five',name='5h',windowMinutes=300,usedPercent=10,groupId=group),dict(id='week',name='7d',windowMinutes=10080,usedPercent=20,groupId=group)])
        for plan in ['plus','pro']:
            cases.append(dict(name=account+kind+group+plan,credits=kind=='credits',group=group,quota={**quota,'plan':plan},full=select(full[field]),display=select(display[field])))
    fixture = root/'fixture.json'
    fixture.write_text(json.dumps(cases))
    subprocess.run(['node','-e',r'''
const fs=require('fs'),vm=require('vm'),assert=require('assert/strict');
const context=vm.createContext({window:{}});
for(const file of ['contracts.js','i18n-catalog.js','i18n.js','ui-state.js'])vm.runInContext(fs.readFileSync('apps/desktop/web/'+file,'utf8'),context);
const ui=context.window.AieyesUI,cases=JSON.parse(fs.readFileSync(process.argv[1],'utf8'));
for(const row of cases){
 const entries=rows=>JSON.stringify(ui.estimateEntries(rows,row.credits,row.quota,row.group).map(e=>[e.field,e.value,e.record.id,e.historical]));
 assert.equal(entries(row.full),entries(row.display),row.name);
 assert.equal(ui.canonicalEstimates(row.full)[0]?.id,ui.canonicalEstimates(row.display)[0]?.id,row.name+' latest');
}
console.log('Web full/display golden selections passed:',cases.length);
''',str(fixture)],cwd=ROOT,check=True)
    if sys.platform == 'darwin':
        verifier = root/'verify.swift'
        verifier.write_text('''import Foundation
struct Case: Decodable { var name: String, credits: Bool, group: String, quota: Quota, full: [QuotaEstimate], display: [QuotaEstimate] }
@main struct Verify {
 static func main() throws {
  let cases = try JSONDecoder().decode([Case].self, from: Data(contentsOf: URL(fileURLWithPath:CommandLine.arguments[1])))
  for row in cases {
   func entries(_ records:[QuotaEstimate]) -> [String] { EstimatePresentation.entries(records,credits:row.credits,quota:row.quota,group:row.group.isEmpty ? nil : row.group).map { "\\($0.id):\\($0.value):\\($0.record.id):\\($0.historical)" } }
   precondition(entries(row.full)==entries(row.display),row.name)
   precondition(EstimatePresentation.canonical(row.full).first?.id==EstimatePresentation.canonical(row.display).first?.id)
  }
  print("Swift full/display golden selections passed: \\(cases.count)")
 }
}
''')
        executable=root/'verify'
        subprocess.run(['swiftc',str(ROOT/'apps/macos/Sources/Aieyes/Localization.swift'),str(ROOT/'apps/macos/Sources/Aieyes/LocalizationCatalog.swift'),'-module-cache-path',str(ROOT/'.build/swift-module-cache'),str(ROOT/'apps/macos/Sources/Aieyes/MonitorSelection.swift'),str(ROOT/'apps/macos/Sources/Aieyes/Models.swift'),str(verifier),'-o',str(executable)],check=True)
        subprocess.run([str(executable),str(fixture)],check=True)
