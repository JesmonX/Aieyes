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
 const estimateRow=detail.locator('.supplement-facts .estimate-row');assert(!/容量倍率/.test(await estimateRow.innerText()),`${surface} ${width} ${style}: estimate row must not name the capacity ratio`);assert.equal(await estimateRow.evaluate(e=>getComputedStyle(e.querySelector('strong')).textAlign),'right',`${surface} ${width} ${style}: estimate values must align right`);
 assert(await detail.evaluate(e=>e.scrollWidth<=e.clientWidth+1),`${surface} ${width} ${style}: detail overflow`);
 if(surface==='floating.html' && width===450 && style==='tile'){assert((await detail.boundingBox()).height<320,'Narrow details with the analysis action row should remain compact');await page.screenshot({path:'.local/quota-supplement.png',fullPage:true});}
 await detail.locator('summary').click();assert.equal(await page.locator('[data-ring-detail]').count(),0);
 await tiles.nth(1).click();assert.equal(await page.locator('[data-ring-detail] .orb-context strong').innerText(),'Claude and GPT Models');
 await page.locator('[data-ring-detail] [data-estimate]').first().click();assert.equal(await page.locator('#estimate-window').inputValue(),'Claude and GPT Models:300');await page.locator('#quota-close').click();
 await page.locator('[data-ring-detail] summary').click();
 await page.evaluate(()=>{state.provider='codex';state.accountKey='';const q=state.dashboard.quotas.find(q=>q.provider==='codex');q.identity={email:'layout@example.test'};q.windows=[300,10080].map(m=>({id:String(m),name:m===300?'5h':'7d',windowMinutes:m,usedPercent:59,resetsAt:Date.now()/1000+3600}));q.bankReset={availableCount:2,credits:[{id:'test',title:'Reset Credit',status:'available',expiresAt:1900000000},{id:'test2',title:'Weekly Reset',status:'available',expiresAt:1900000100}]};render();});
 await page.locator('button[data-quota-account="codex:c"]').click();
 const bank=page.locator('[data-agent-detail="ring-bank:codex:c"]');assert.equal(await bank.evaluate(e=>e.open),false);await bank.locator('summary').click();assert.match(await bank.innerText(),/Reset Credit/);
 if(surface==='index.html'&&width===1120&&style==='tile'){const boxes=await bank.locator('.bank-items > div').evaluateAll(es=>es.map(e=>{const r=e.getBoundingClientRect();return {x:r.x,y:r.y};}));assert(Math.abs(boxes[0].y-boxes[1].y)<2&&boxes[1].x>boxes[0].x,'Bank Reset cards should share a row when the card is wide');}
 await bank.locator('summary').click();
 const balance=page.locator('.credit-value-row');assert.match(await balance.innerText(),/1000.*200/);assert.equal(await balance.evaluate(e=>getComputedStyle(e.querySelector('strong')).textAlign),'right','Balance values must align right');assert.equal(await page.locator('[data-agent-detail="ring-credits:codex:c"]').count(),0);await balance.click();await page.locator('#quota-close').click();
 const periods=page.locator('[data-ring-detail] .supplement-period');assert.equal(await periods.first().locator('.period-heading > span').innerText(),'5h');assert(!/剩余/.test(await periods.first().locator('.period-heading').innerText()));assert(!/\//.test(await periods.first().locator('time').innerText()));
 if(surface==='floating.html' && width===450 && style==='tile'){await page.evaluate(async()=>{await Promise.all(document.getAnimations().map(a=>a.finished.catch(()=>{})));});await page.screenshot({path:'.local/quota-supplement-codex.png',fullPage:true});}
 assert.equal(await page.locator('[data-ring-detail] .supplement-facts [data-view-analysis]').count(),1);
 assert.equal(await page.locator('[data-ring-detail] .orb-context [data-view-analysis]').count(),0);
 const layout = await page.locator('[data-ring-detail]').evaluate(e=>{const box=e.getBoundingClientRect(),grid=e.parentElement.getBoundingClientRect(),primary=e.querySelector('.supplement-primary').getBoundingClientRect(),facts=e.querySelector('.supplement-facts').getBoundingClientRect(),bank=e.querySelector('.supplement-disclosure').getBoundingClientRect();return {width:box.width,grid:grid.width,primaryX:primary.x,factsX:facts.x,primaryBottom:primary.bottom,factsBottom:facts.bottom,bankY:bank.y,bankWidth:bank.width};});
 assert(Math.abs(layout.width-layout.grid)<2,'Expanded card must fill its row');
 assert(layout.bankY>=Math.max(layout.primaryBottom,layout.factsBottom)-1,'Bank Reset must follow both groups');
 assert(layout.bankWidth>=layout.width-24,'Bank Reset must span the card');
 if(layout.width>=622)assert(layout.factsX>layout.primaryX+100,'Wide details must arrange groups side by side');
 const stretch=await page.locator('[data-ring-detail]').evaluate(e=>{
   const primary=e.querySelector('.supplement-primary'),facts=e.querySelector('.supplement-facts'),periods=[...primary.querySelectorAll('.supplement-period')],before=periods.map(p=>p.getBoundingClientRect().height),template=facts.querySelector('.analysis-row'),added=[];
   for(let i=0;i<6;i++){const row=template.cloneNode(true);facts.append(row);added.push(row);}
   const grown=periods.map(p=>p.getBoundingClientRect().height),primaryBottom=primary.getBoundingClientRect().bottom,lastBottom=periods.at(-1).getBoundingClientRect().bottom;
   added.forEach(row=>row.remove());
   return {stretch:getComputedStyle(primary).alignContent==='stretch',before,grown,restored:periods.map(p=>p.getBoundingClientRect().height),primaryBottom,lastBottom};
 });
 if(layout.width>=622){assert(stretch.stretch,'Wide quota bars must stretch with the facts column');assert(stretch.grown.every((h,i)=>h>stretch.before[i]+10),'Wide quota bars must grow when the facts column is taller');assert(Math.abs(stretch.lastBottom-stretch.primaryBottom)<4,'Quota bars must fill the stretched column');assert(stretch.restored.every((h,i)=>Math.abs(h-stretch.before[i])<1),'Quota bars must keep their natural height when facts are shorter');}
 else assert(!stretch.stretch,'Stacked quota bars must keep their natural height');
 if(surface==='index.html' && width===1120 && style==='tile'){await page.locator('#quota-close').waitFor({state:'hidden'});await page.screenshot({path:'.local/reflow-main.png',fullPage:true});}

 await page.evaluate(()=>{const q=state.dashboard.quotas.find(q=>q.provider==='codex');document.querySelector('[data-ring-detail]').outerHTML=quotaSupplement({...q,windows:q.windows.slice(0,1)});});
 assert(await page.locator('.supplement-periods').evaluate(e=>Math.abs(e.firstElementChild.getBoundingClientRect().width-e.getBoundingClientRect().width)<2),'A single quota must fill the primary column');
 if(surface==='index.html' && width===1120 && style==='tile')await page.screenshot({path:'.local/detail-single-main.png',fullPage:true});
 await page.evaluate(()=>{const q={...agyFixture.q,provider:'qoder-cn',accountId:'cn',name:'Qoder CN',plan:'pro',windows:[],meters:[{id:'userQuota',label:'Plan Credits',unit:'credits',used:25,total:2000,remaining:1975,resetsAt:1900000000},{id:'addOnQuota',label:'Addon Credits',unit:'credits',used:2,total:20,remaining:18}]};window.qoderFixture=q;document.querySelector('.quota-rings').outerHTML=AieyesQuotaRings.gridHTML([{account:{id:'cn',provider:'qoder-cn'},quota:q}]);});
 assert.equal(await page.locator('.tile-credit-facts,[data-ring-reset]').count(),0);
 if(style==='dial') {
   assert.deepEqual(await page.locator('.qoder-credit-value .ring-swap-time').allTextContents(),['18','1975']);
   await page.locator('.quota-tile').hover();
   assert.deepEqual(await page.locator('.qoder-credit-value .ring-swap-pct').allTextContents(),['90%','99%']);
   assert(await page.locator('.qoder-credit-value .ring-swap-pct').first().isVisible());
   assert(!await page.locator('.qoder-credit-value .ring-swap-time').first().isVisible());
 }
 else {assert.deepEqual(await page.locator('.qoder-credit-ratio').allTextContents(),['18/20','1975/2000']);assert.deepEqual(await page.locator('.ring-num').allTextContents(),['90','99']);}
 assert.equal(await page.locator('.ring[data-kind="duo"] .ring-track').count(),2);
 assert.deepEqual(await page.evaluate(()=>{qoderFixture.meters.reverse();const r=AieyesQuotaRings.ring(qoderFixture);return [r.kind,r.top.meter.id,r.top.remaining,r.bottom.meter.id,r.bottom.remaining];}),['duo','addOnQuota',90,'userQuota',98.75]);
 assert.deepEqual(await page.evaluate(()=>{const r=AieyesQuotaRings.ring({...qoderFixture,provider:'qoder'});return [r.kind,r.top.remaining,r.bottom.remaining];}),['duo',90,98.75]);
 assert.equal(await page.evaluate(()=>AieyesQuotaRings.ring({...qoderFixture,meters:qoderFixture.meters.map(m=>m.id==='addOnQuota'?{...m,total:0}:m)}).top.remaining),0);
 assert.equal(await page.evaluate(()=>{qoderFixture.meters.find(m=>m.id==='addOnQuota').remaining=0;return AieyesQuotaRings.qoderCreditSum(qoderFixture);}), '1975');
 assert.equal(await page.evaluate(()=>AieyesQuotaRings.ring({...qoderFixture,meters:qoderFixture.meters.filter(m=>m.id!=='userQuota')}).kind),'empty');
 for(const label of await page.locator('.qoder-credit-value,.qoder-credit-ratio').all())assert(await label.evaluate(e=>e.scrollWidth<=e.clientWidth+1),`${width} ${style}: Qoder credit text clipped`);
 if(surface==='floating.html' && width===450){await page.evaluate(async()=>{await Promise.all(document.getAnimations().map(a=>a.finished.catch(()=>{})));});await page.screenshot({path:'.local/qoder-'+style+'.png',fullPage:true});}
 await page.evaluate(()=>{const q={...qoderFixture,provider:'deepseek',accountId:'ds',name:'DeepSeek',meters:[],balances:[{currency:'CNY',total:'100',granted:'25',toppedUp:'75'}]};window.dsFixture=q;document.querySelector('.quota-rings').outerHTML=AieyesQuotaRings.gridHTML([{account:{id:'ds',provider:'deepseek'},quota:q}]);});
 assert.equal(await page.evaluate(()=>AieyesQuotaRings.ring(dsFixture).split.share),.25);
 assert.equal(await page.locator('.ring-granted').count(),1);
 assert.equal(await page.locator('.ring-paid').count(),1);
 assert.match(await page.locator('.balance-granted').textContent(),/¥25/);
 assert.match(await page.locator('.balance-paid').textContent(),/¥75/);
 for(const amounts of [[0,100],[25,0],[-1,100],['bad',100]]) {
   const split=await page.evaluate(([g,p])=>AieyesQuotaRings.ring({...dsFixture,balances:[{currency:'CNY',total:'100',granted:String(g),toppedUp:String(p)}]}).split,amounts);
   assert.equal(Boolean(split),amounts[0]===25);
 }
 assert(await page.locator('.quota-rings').evaluate(e=>e.scrollWidth<=e.clientWidth+1),`DeepSeek overflow: ${surface} ${width} ${style}`);
 if(surface==='floating.html' && width===450)await page.screenshot({path:'.local/deepseek-'+style+'.png',fullPage:true});
 await page.evaluate(()=>{const q={...qoderFixture,meters:qoderFixture.meters.map(m=>m.id==='addOnQuota'?{...m,remaining:18}:m)};document.querySelector('.quota-rings').innerHTML=quotaSupplement(q);});
 assert.equal(await page.locator('[data-ring-detail] .detail-ring').count(),0);
 assert.equal(await page.locator('[data-ring-detail] .meter-bar.supplement-period').count(),2);
 assert.equal(await page.locator('[data-ring-detail] .meter-bar > strong').count(),2);
 assert(!/未提供/.test((await page.locator('[data-ring-detail] .period-heading').allInnerTexts()).join(' ')),'Qoder without reset times must not print a missing-time label');
 const meterBoxes=await page.locator('[data-ring-detail] .meter-bar').evaluateAll(es=>es.map(e=>{const r=e.getBoundingClientRect();return {x:r.x,y:r.y};}));
 assert(Math.abs(meterBoxes[0].y-meterBoxes[1].y)<2 && meterBoxes[1].x>meterBoxes[0].x,'Qoder buckets should share the same row like two time windows');
 assert.equal(await page.locator('[data-ring-detail] .supplement-facts [data-view-analysis]').count(),1);
 assert(await page.locator('[data-ring-detail]').evaluate(e=>e.scrollWidth<=e.clientWidth+1));
 if(surface==='index.html' && width===1120 && style==='tile')await page.screenshot({path:'.local/reflow-qoder-main.png',fullPage:true});
 // Kiro has an independent ring and reset for every credit bucket.
 await page.evaluate(()=>{
   const at=Date.now()/1000;
   const q={...qoderFixture,provider:'kiro',accountId:'kiro-fixture',name:'Kiro',meters:[
     {id:'plan',label:'套餐 Credits',unit:'credits',used:24.75,total:2000,remaining:1975.25,resetsAt:at+86400},
     {id:'bonus',label:'赠送 Credits',unit:'credits',used:25,total:100,remaining:75,resetsAt:at+7*86400},
     {id:'overage',label:'超额 Credits',unit:'credits',used:12.5,resetsAt:at+2*86400}
   ]};
   window.kiroFixture=q;state.dashboard.quotas.push(q);
   document.querySelector('.quota-rings').outerHTML=AieyesQuotaRings.gridHTML([{account:{id:q.accountId,provider:q.provider},quota:q}]);
 });
 const kiroTiles=page.locator('button[data-quota-account="kiro:kiro-fixture"]');
 assert.equal(await kiroTiles.count(),3);
 assert.equal(await kiroTiles.locator('.ring[data-kind=single]').count(),2);
 assert.equal(await kiroTiles.locator('[data-ring-reset]').count(),3,'Every bucket keeps its own reset, including unknown capacity');
 assert.deepEqual(await kiroTiles.locator('[data-ring-reset]').allTextContents(),['1d','7d','2d']);
 if(style==='dial'){
   assert.deepEqual(await kiroTiles.locator('.meter-credit-value .ring-swap-time').allTextContents(),['1975.25','75','—']);
   await kiroTiles.first().focus();assert(await kiroTiles.first().locator('.ring-swap-pct').isVisible());
 } else assert.deepEqual(await kiroTiles.locator('.meter-credit-ratio').allTextContents(),['1975.25/2000','75/100','—/—']);
 assert(await page.locator('.quota-rings').evaluate(e=>e.scrollWidth<=e.clientWidth+1),`Kiro overflow: ${surface} ${width} ${style}`);
 for(const label of await kiroTiles.locator('.meter-credit-ratio,.ring-center,.tile-times').all())assert(await label.evaluate(e=>e.scrollWidth<=e.clientWidth+1),`Kiro clipped: ${surface} ${width} ${style}`);
 if(surface==='floating.html'&&width===450){await page.evaluate(async()=>{await Promise.all(document.getAnimations().map(a=>a.finished.catch(()=>{})));});await page.screenshot({path:'.local/kiro-'+style+'-collapsed.png',fullPage:true});}
 await kiroTiles.nth(1).click();
 assert.equal(await page.locator('[data-ring-detail] .meter-bar').count(),3);
 assert.equal(await page.locator('[data-ring-detail] .detail-ring').count(),0);
 assert.equal(await page.locator('[data-ring-detail] time').count(),3);
 assert(await page.locator('[data-ring-detail]').evaluate(e=>e.scrollWidth<=e.clientWidth+1));
 const selected=await kiroTiles.nth(1).getAttribute('data-quota-tile');
 await page.evaluate(()=>{kiroFixture.meters.reverse();AieyesQuotaRings.replace(kiroFixture);});
 assert.equal(await page.locator('[aria-expanded=true][data-quota-tile]').getAttribute('data-quota-tile'),selected,'Refreshing or reordering buckets preserves the open ring');
 if(surface==='floating.html'&&width===450){await page.evaluate(async()=>{await Promise.all(document.getAnimations().map(a=>a.finished.catch(()=>{})));});await page.screenshot({path:'.local/kiro-'+style+'-expanded.png',fullPage:true});}
 await page.locator('[data-ring-detail] summary').click();assert.equal(await page.locator('[data-ring-detail]').count(),0);
 assert.equal(await page.evaluate(()=>AieyesQuotaRings.models({...kiroFixture,meters:kiroFixture.meters.slice(0,1)}).length),1);
 assert.equal(await page.evaluate(()=>AieyesQuotaRings.models({...kiroFixture,meters:[]})[0].ring.kind),'empty');
 assert.deepEqual(errors,[]);await page.close();
 } console.log('Compact supplement: both surfaces and styles at 320/450/1120px; full identity, group names, estimates, close and Qoder balances passed.'); }
 finally {await browser.close();server.close();}
})().catch(e=>{console.error(e);process.exit(1);});
