const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const sandbox={window:{},Date,localStorage:{getItem(){return null;},setItem(){}}};
vm.createContext(sandbox);
vm.runInContext(fs.readFileSync(path.resolve(__dirname,'../apps/desktop/web/ui-state.js'),'utf8'),sandbox);
const ui=sandbox.window.AieyesUI,now=1800000000;
for(const [seconds,text] of [[1,'1 分后重置'],[59*60,'59 分后重置'],[59*60+1,'1 小时后重置'],[3600,'1 小时后重置'],[86400,'1 天后重置'],[26*3600+15*60,'1 天 2 小时后重置'],[7*86400,'7 天后重置'],[23*3600+59*60,'23 小时 59 分后重置'],[86400+60,'1 天后重置'],[86400+3600+60,'1 天 1 小时后重置']])assert.equal(ui.resetText(now+seconds,now),text);
for(const [windowMinutes,text] of [[300,'5 小时后重置'],[10080,'7 天后重置']]) {
  const snapshot={resetsAt:now-1,windowMinutes},original=JSON.stringify(snapshot);
  for(const elapsed of [0,60,30*86400]) {
    assert.equal(ui.resetText(snapshot.resetsAt,now+elapsed,windowMinutes),text);
    assert.equal(ui.resetDisplayTime(snapshot.resetsAt,windowMinutes,now+elapsed),now+elapsed+windowMinutes*60);
    assert.equal(ui.resetText(null,now+elapsed,windowMinutes),text);
    assert.match(ui.quotaReset(snapshot,now+elapsed),new RegExp('^'+text+' · \\d{2}/\\d{2} \\d{2}:\\d{2}$'));
  }
  assert.equal(JSON.stringify(snapshot),original);
  assert.equal(ui.resetText(now+120*60,now,windowMinutes),'2 小时后重置');
  assert.equal(ui.resetDisplayTime(now+120*60,windowMinutes,now),now+120*60);
  assert.match(ui.quotaReset({resetsAt:now+120*60,windowMinutes},now),/^2 小时后重置 · \d{2}\/\d{2} \d{2}:\d{2}$/);
}
for(const value of [null,undefined,0,NaN,Infinity])assert.equal(ui.resetText(value,now),'重置时间未知');
assert.equal(ui.resetText(null,now,-1),'重置时间未知');
assert.equal(ui.resetText(now-1,now),'确认重置中');
assert.equal(ui.subscription('plus'),'Plus');assert.equal(ui.subscription(' pro '),'Pro');assert.equal(ui.subscription('Special Plan'),'Special Plan');
for(const provider of ['codex','claude','antigravity','deepseek']) {
  const png=fs.readFileSync(path.resolve(__dirname,'../apps/desktop/web/provider-'+provider+'.png'));
  assert.equal(png.subarray(0,8).toString('hex'),'89504e470d0a1a0a');
}
const windows=[{resetsAt:now+2*3600,windowMinutes:300,usedPercent:0},{resetsAt:now+2*86400,windowMinutes:10080,usedPercent:20}];
const original=JSON.stringify(windows);
for(const elapsed of [0,60,3600]) {
  assert.match(ui.quotaReset(windows[0],now+elapsed),/^5 小时后重置 · /);
  assert.equal(ui.resetDisplayTime(windows[0].resetsAt,300,now+elapsed,0),now+elapsed+5*3600);
  assert.equal(ui.resetDisplayTime(windows[1].resetsAt,10080,now+elapsed,20),windows[1].resetsAt);
}
assert.match(ui.quotaReset(windows[1],now+3600),/^1 天 23 小时后重置/);
assert.equal(JSON.stringify(windows),original);
for(const usedPercent of [undefined,null,NaN,Infinity,-1,0.001,20]) {
  assert.match(ui.quotaReset({...windows[0],usedPercent},now),/^2 小时后重置/);
}
for(const usedPercent of [0,0.001,10,0]) {
  assert.match(ui.quotaReset({...windows[0],usedPercent},now+3600),usedPercent===0?/^5 小时后重置/:/^1 小时后重置/);
}
assert.match(ui.quotaReset({...windows[1],usedPercent:0},now+3600),/^7 天后重置/);
assert.match(ui.quotaReset({...windows[0],windowMinutes:null},now),/^2 小时后重置/);
console.log('Web reset durations, dormant windows, dates, refresh transitions, subscription names and bundled icons passed');
