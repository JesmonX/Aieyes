const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
function fixture(){
  window.setInterval=()=>0;
  const at=Date.now()/1000,groups=['Gemini Models','Claude and GPT Models','Future Models','Gemini 4 Argon'];
  const q={provider:'antigravity',accountId:'legacy-shared',sourceId:'agy-source',name:'Antigravity',identity:{key:'google:fixture',email:'a.very.long.account.name.for.layout.verification@example.test',subscription:'Google AI Ultra',checkedAt:at,stale:false},plan:'Google AI Ultra',updatedAt:at,origin:'live',windows:groups.flatMap(group=>(group==='Gemini 4 Argon'?[10080]:[300,10080]).map(minutes=>({id:group+':'+minutes,name:group+' · '+(minutes===300?'5h':'7d'),groupId:group,groupName:group,windowMinutes:minutes,usedPercent:20,resetsAt:at+minutes*60})))};
  const codex={...q,provider:'codex',accountId:'c',sourceId:'c',name:'Codex',plan:'pro',windows:[],credits:{hasCredits:true,unlimited:false,balance:'1000'}};
  const record=(group,value)=>({id:group,accountKey:'antigravity:legacy-shared',groupId:group,windowId:group+':300',windowName:group+' · 5h',valuationMode:'fiveHour',status:'completed',sourceIds:['agy-source'],sourceNames:['CLI'],startedAt:at-100,checkpointAt:at,consumedPercent:10,cost:1,totalTokens:100,pricedTokens:100,fiveHourValue:value,weeklyValue:value*6,weeklyDirectValue:value*6,weeklyRatioValue:value*5,calculationNote:'分组估值',prices:[]});
  const settings={version:2,accountAliases:{'agy:shared':'antigravity:legacy-shared'},accounts:[q,codex].map(q=>({id:q.accountId,name:q.name,provider:q.provider,quotaEnabled:true})),sources:[q,codex].map(q=>({id:q.sourceId,provider:q.provider,accountId:q.accountId,name:q.name,path:'/fixture',enabled:true})),hosts:[],modelMappings:{},proxy:{mode:'direct',url:''},refreshSeconds:300,serverRefreshSeconds:10};
  const empty={tokens:{input:0,output:0,cacheRead:0,cacheWrite:0},total:0,cost:0,pricedTokens:0,events:0};
  window.agyFixture={settings,q,records:[record(groups[0],10),record(groups[1],100),record(groups[3],777)],calls:[]};
  localStorage.setItem('aieyes.panel.accounts.v2',JSON.stringify({version:2,providers:{agy:{mode:'custom',keys:['agy:shared']}}}));
  localStorage.setItem('quota.expanded.v2.panel.agy:shared','false');
  localStorage.setItem('aieyes.panel.view',JSON.stringify({provider:'agy',accountKey:'agy:shared'}));
  window.__TAURI__={event:{async listen(){return ()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return ()=>{};}})},core:{async invoke(command,args){
    if(command==='desktop_info')return {platform:'windows',mode:'floating',effectiveMode:'floating',panelOpen:true,sessions:[]};
    if(command!=='engine_call')return {};
    const f=agyFixture,{method,params}=args;f.calls.push({method,params});
    if(method==='hello')return {version:'test'};if(method==='settings.get')return settings;
    if(method==='dashboard')return {generatedAt:at,summary:empty,quotas:[q,codex],quotaOrder:['antigravity:legacy-shared','codex:c'],quotaEstimates:f.records,automaticEstimateSummaries:f.summaries??[],creditEstimates:[{...record('',1),id:'credit',accountKey:'codex:c',valuePer1000:200}],modelOptions:[],models:[],dayModels:[],trendDays:[],heatmap:[],sources:[],pricingGaps:[]};
    if(method==='quotaEstimates.start'){const group=q.windows.find(w=>w.id===params.windowId).groupId;f.records.unshift({...record(group,15),id:'active',status:'active'});return f.records[0];}
    if(method==='quotaEstimates.stop'){f.records.find(r=>r.id===params.id).status='completed';return {};}
    return [];
  }}};
}
(async()=>{
 const root=path.resolve(__dirname,'../apps/desktop/web'),server=http.createServer(require('./ui-test-helpers.cjs').serveWebRoot(root));
 await new Promise(r=>server.listen(0,'127.0.0.1',r));const browser=await chromium.launch({headless:true});
 try { for(const surface of ['index.html','floating.html']) for(const width of [320,450,1120]) for(const style of ['dial','tile']) {
 const page=await browser.newPage({viewport:{width,height:900}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);
 await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);await page.evaluate(()=>AieyesApp.ready);
 await page.evaluate(style=>{AieyesQuotaRings.setStyle(document.querySelector('#panel')?'panel':'detail',style);render();},style);
 const tiles=page.locator('button[data-quota-account="antigravity:legacy-shared"]');await tiles.first().waitFor();
 for(const name of ['Geminis','C&Gs','Argon']) {const label=page.locator('.tile-name > span:last-child,.tile-head-name').filter({hasText:new RegExp('^'+name.replace('&','&')+'$')});assert.equal(await label.count(),1);assert(await label.evaluate(e=>e.scrollWidth<=e.clientWidth+1),`${surface} ${width} ${style} clipped ${name}`);}
 await tiles.first().click();const detail=page.locator('[data-ring-detail]');await detail.waitFor();assert.match(await detail.innerText(),/a.very.long.account.name.for.layout.verification@example.test/);assert.equal(await detail.locator('[data-estimate]').count(),1);assert.equal(await detail.locator('.supplement-identity').count(),1);assert.equal(await detail.locator('.supplement-period').count(),2);assert.equal(await detail.locator('.detail-ring svg').count(),0);assert.equal(await detail.locator('.quota-fact,.estimate-entry').count(),0);
 assert(await detail.evaluate(e=>e.scrollWidth<=e.clientWidth+1),`${surface} ${width} ${style}: detail overflow`);
 if(surface==='floating.html' && width===450 && style==='tile'){assert((await detail.boundingBox()).height<290,'Expanded details should remain compact');await page.screenshot({path:'.local/quota-supplement.png',fullPage:true});}
 await detail.locator('summary').click();assert.equal(await page.locator('[data-ring-detail]').count(),0);
 await tiles.nth(1).click();assert.equal(await page.locator('[data-ring-detail] .orb-context strong').innerText(),'Claude and GPT Models');
 await page.locator('[data-ring-detail] [data-estimate]').first().click();assert.equal(await page.locator('#estimate-window').inputValue(),'Claude and GPT Models:300');await page.locator('#quota-close').click();
 await page.locator('[data-ring-detail] summary').click();
 await page.evaluate(()=>{state.provider='codex';state.accountKey='';const q=state.dashboard.quotas.find(q=>q.provider==='codex');q.identity={email:'layout@example.test'};q.windows=[300,10080].map(m=>({id:String(m),name:m===300?'5h':'7d',windowMinutes:m,usedPercent:59,resetsAt:Date.now()/1000+3600}));q.bankReset={availableCount:1,credits:[{id:'test',title:'Reset Credit',status:'available',expiresAt:1900000000}]};render();});
 await page.locator('button[data-quota-account="codex:c"]').click();
 const bank=page.locator('[data-agent-detail="ring-bank:codex:c"]');assert.equal(await bank.evaluate(e=>e.open),false);await bank.locator('summary').click();assert.match(await bank.innerText(),/Reset Credit/);await bank.locator('summary').click();
 const balance=page.locator('.credit-value-row');assert.match(await balance.innerText(),/1000.*200/);assert.equal(await page.locator('[data-agent-detail="ring-credits:codex:c"]').count(),0);await balance.click();await page.locator('#quota-close').click();
 const periods=page.locator('[data-ring-detail] .supplement-period');assert.equal(await periods.first().locator('.period-heading > span').innerText(),'5h');assert(!/剩余/.test(await periods.first().locator('.period-heading').innerText()));assert(!/\//.test(await periods.first().locator('time').innerText()));
 if(surface==='floating.html' && width===450 && style==='tile'){await page.evaluate(async()=>{await Promise.all(document.getAnimations().map(a=>a.finished.catch(()=>{})));});await page.screenshot({path:'.local/quota-supplement-codex.png',fullPage:true});}
 await page.evaluate(()=>{const q={...agyFixture.q,provider:'qoder-cn',accountId:'cn',name:'Qoder CN',plan:'pro',windows:[],meters:[{id:'userQuota',label:'Plan Credits',unit:'credits',used:25,total:2000,remaining:1975,resetsAt:1900000000},{id:'addOnQuota',label:'Addon Credits',unit:'credits',used:2,total:20,remaining:18}]};window.qoderFixture=q;document.querySelector('.quota-rings').outerHTML=AieyesQuotaRings.gridHTML([{account:{id:'cn',provider:'qoder-cn'},quota:q}]);});
 assert.equal(await page.locator('.tile-credit-facts,[data-ring-reset]').count(),0);
 if(style==='dial')assert.equal(await page.locator('.qoder-credit-sum').innerText(),'1975+18');
 else {assert.deepEqual(await page.locator('.qoder-credit-ratio').allTextContents(),['18/20','1975/2000']);assert.deepEqual(await page.locator('.ring-num').allTextContents(),['90','99']);}
 assert.equal(await page.locator('.ring[data-kind="duo"] .ring-track').count(),2);
 assert.deepEqual(await page.evaluate(()=>{qoderFixture.meters.reverse();const r=AieyesQuotaRings.ring(qoderFixture);return [r.kind,r.top.meter.id,r.top.remaining,r.bottom.meter.id,r.bottom.remaining];}),['duo','addOnQuota',90,'userQuota',98.75]);
 assert.deepEqual(await page.evaluate(()=>{const r=AieyesQuotaRings.ring({...qoderFixture,provider:'qoder'});return [r.kind,r.top.remaining,r.bottom.remaining];}),['duo',90,98.75]);
 assert.equal(await page.evaluate(()=>AieyesQuotaRings.ring({...qoderFixture,meters:qoderFixture.meters.map(m=>m.id==='addOnQuota'?{...m,total:0}:m)}).top.remaining),0);
 assert.equal(await page.evaluate(()=>{qoderFixture.meters.find(m=>m.id==='addOnQuota').remaining=0;return AieyesQuotaRings.qoderCreditSum(qoderFixture);}), '1975');
 assert.equal(await page.evaluate(()=>AieyesQuotaRings.ring({...qoderFixture,meters:qoderFixture.meters.filter(m=>m.id!=='userQuota')}).kind),'empty');
 for(const label of await page.locator('.qoder-credit-sum,.qoder-credit-ratio').all())assert(await label.evaluate(e=>e.scrollWidth<=e.clientWidth+1),`${width} ${style}: Qoder credit text clipped`);
 if(surface==='floating.html' && width===450){await page.evaluate(async()=>{await Promise.all(document.getAnimations().map(a=>a.finished.catch(()=>{})));});await page.screenshot({path:'.local/qoder-'+style+'.png',fullPage:true});}
 assert.deepEqual(errors,[]);await page.close();
 } console.log('Compact supplement: both surfaces and styles at 320/450/1120px; full identity, group names, estimates, close and Qoder balances passed.'); }
 finally {await browser.close();server.close();}
})().catch(e=>{console.error(e);process.exit(1);});
