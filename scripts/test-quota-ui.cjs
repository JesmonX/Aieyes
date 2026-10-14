const {discardEditor}=require('./ui-test-helpers.cjs');
// End-to-end quota UI flows on both desktop surfaces, with isolated in-memory IPC.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
function fixture(){
  window.quotaTimers=[];window.setInterval=(fn,ms)=>{quotaTimers.push({fn,ms});return quotaTimers.length;};
  const now=Math.floor(Date.now()/1000),tokens={input:0,output:0,cacheRead:0,cacheWrite:0};
  const summary={tokens,total:0,cost:0,pricedTokens:0,events:0};
  const win=(id,name,minutes,groupName)=>({id,name,windowMinutes:minutes,usedPercent:35,resetsAt:now+10000,groupName});
  const rows=[{provider:'codex',accountId:'c',name:'个人订阅',plan:'pro',sourceId:'c',updatedAt:now,origin:'live',windows:[win('weekly','7d',10080)]},{provider:'agy',accountId:'a',name:'工作空间 agy',sourceId:'a',updatedAt:now,origin:'live',windows:['Claude Opus','Claude Sonnet','Gemini Pro'].flatMap(g=>[win(g+':5h',g+' · 5h',300,g),win(g+':7d',g+' · 7d',10080,g)])}];
  const settings={accounts:rows.map(q=>({id:q.accountId,name:q.name,provider:q.provider,quotaEnabled:true})),sources:rows.map(q=>({id:q.accountId,name:q.name,provider:q.provider,accountId:q.accountId,enabled:true,path:'/fixture'})),hosts:[],modelMappings:{},proxy:{mode:'system',url:''},refreshSeconds:300,serverRefreshSeconds:10};
  settings.sources[0].accountId='';settings.sources[0].codexHomeId='fixture-home';settings.accounts[0].quotaProfileId='fixture-home:profile';
  window.quotaFixture={rows,settings,order:['codex:c','antigravity:a'],records:[],calls:[],fail:false,panelOpen:true};
  const info=()=>({platform:'windows',material:'opaque',mode:'floating',effectiveMode:'floating',sessions:[],summary:'暂无活跃会话',panelOpen:window.quotaFixture.panelOpen,panelPinned:true,hidden:false});
  window.__TAURI__={event:{async listen(){return ()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return ()=>{};}})},core:{async invoke(command,args){
    const f=window.quotaFixture;if(command==='desktop_info')return info();if(command==='desktop_panel'){f.panelOpen=args.open;return info();}if(command!=='engine_call')return {};
    const {method,params}=args;f.calls.push({method,params});
    if(method==='hello')return {version:'test'};if(method==='settings.get')return structuredClone(settings);
    if(method==='dashboard'||method==='dashboard.summary')return {generatedAt:now,summary,quotas:[...rows].sort((a,b)=>f.order.indexOf(AieyesUI.canonicalProvider(a.provider)+':'+a.accountId)-f.order.indexOf(AieyesUI.canonicalProvider(b.provider)+':'+b.accountId)),quotaOrder:f.order,quotaEstimates:f.records,modelOptions:[],models:[],dayModels:[],trendDays:[],heatmap:[],sources:settings.sources.map(s=>({...s,autoValuationStatus:s.autoValuation?.enabled?{state:'collecting',note:'等待有效样本',accountKey:'codex:c'}:null})),pricingGaps:[]};
    if(method==='autoEstimates.configure'){if(f.failAutomatic)throw new Error('模拟自动估值保存失败');const src=settings.sources.find(s=>s.id===params.sourceId);if(params.enabled&&!params.confirmed)throw new Error('请同意');src.autoValuation=params.enabled?{enabled:true,consentVersion:1}:null;return {settings:structuredClone(settings)};}
    if(method==='sources.scan')return [];if(method==='quotas.refresh')return rows;
    if(method==='quotas.order.set'){if(f.fail)throw new Error('保存失败，请重试');f.order=params.keys;return f.order;}
    if(method.startsWith('quotaEstimates.')){
      if(f.fail)throw new Error('测试同步失败，采样已保留');
      if(method.endsWith('start')||method.endsWith('restart')){if(!params.confirmed)throw new Error('请确认');if(method.endsWith('.start')&&!params.sourceIds.length)throw new Error('请选择用量数据源');f.records=[{id:'sample',accountKey:'codex:c',windowId:'weekly',windowName:'7d',sourceIds:['c'],sourceNames:['本机'],status:'active',reason:'',startedAt:now-60,checkpointAt:now,consumedPercent:6,cost:1.2,totalTokens:100,pricedTokens:100,weeklyValue:20,calculationNote:'手动采样估值',prices:[]}];}
      if(method.endsWith('.stop'))f.records[0].status='completed';return f.records[0];
    }
    return [];
  }}};
}
(async()=>{
 const root=path.resolve(__dirname,'../apps/desktop/web'),output=path.resolve(__dirname,'../.local/quota-review/web');fs.mkdirSync(output,{recursive:true});
 const server=http.createServer((req,res)=>{if(require('./ui-test-helpers.cjs').serveFont(req,res))return;const file=path.join(root,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404);res.end();return;}res.setHeader('content-type',({'.js':'application/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
 await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
 try{
  browser=await chromium.launch({headless:true});
  for(const surface of ['index.html','floating.html']){
   const page=await browser.newPage({viewport:{width:surface==='floating.html'?450:1120,height:760}}),errors=[];
   page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
   const agy=page.locator('[data-quota="antigravity:a"]:visible');await agy.waitFor({state:'visible'});
   assert.equal(await agy.locator('.agy-group').count(),3);
   if(surface==='floating.html'){
     assert.equal(await agy.locator('.resource-bar:visible').count(),2,'Multiple panel accounts default to key windows');
     assert.equal(await agy.locator('.quota-reset:visible').count(),0);
     await agy.locator('.quota-title').click();
   }
   assert.equal(await agy.locator('.resource-bar:visible').count(),6,'Expanded accounts show all model groups');
   assert.equal(await agy.locator('.quota-reset:visible').count(),6);
   const codex=page.locator('[data-quota="codex:c"]:visible');
   assert.equal(await codex.locator('.subscription-badge').textContent(),'Pro');
   const wasExpanded=await codex.locator('.quota-disclosure').evaluate(el=>el.open);
   if(!wasExpanded)await codex.locator('.quota-title').click();
   const presentation=JSON.parse(fs.readFileSync(path.join(__dirname,'fixtures/estimate-presentation.json'),'utf8'));
   const originalQuota=await page.evaluate(()=>structuredClone(quotaFixture.rows[0]));
   const displayCases=[presentation.cases[0],presentation.cases[3],presentation.cases[4],presentation.cases[1],presentation.cases[10]];
   for(const [index,row] of displayCases.entries()) {
     await page.evaluate(async({row,record})=>{
       Object.assign(quotaFixture.rows[0],{plan:row.quota.plan??'pro',windows:row.quota.windows});
       quotaFixture.records=[{...record,accountKey:'codex:c',sourceIds:['c']}];await loadDashboard();
     },{row,record:presentation.record});
     const entry=codex.locator('[data-estimate]');
     if(index===0)await entry.click();
     await page.waitForFunction(expected=>{
       const rows=[...document.querySelectorAll('#quota-dialog .estimate-result > .between')];
       return rows.length===expected+1;
     },row.fields.length);
     const result=page.locator('#quota-dialog .estimate-result');
     for(const [field,label] of [['fiveHourValue','本段 5h API 等价值'],['weeklyValue','7d 推算 · 同期消耗比例'],['weeklyRatioValue','7d 推算 · 近期容量倍率']])assert.equal(await result.getByText(label,{exact:true}).count(),row.fields.includes(field)?1:0,row.name);
     if(!row.fields.includes('weeklyValue')) {
       assert.doesNotMatch(await result.textContent(),/7d 直接估值|同期消耗比例/);
     }
     if(!row.fields.includes('fiveHourValue')) {
       assert.doesNotMatch(await result.textContent(),/本段 5h|容量倍率|倍率学习中/);
     }
     if(index<2)await result.screenshot({path:path.join(output,`valuation-${index===0?'plus':'weekly'}-${surface}.png`)});
     await page.locator('#quota-close').click();
     await page.waitForFunction(expected=>document.querySelector('[data-estimate="codex:c"]')?.innerText.replace(/\s+/g,' ').trim()===expected,row.summary.replace(/\s+/g,' '));
     const help=await entry.evaluate(el=>el.getAttribute('title')??el.getAttribute('data-hint'));
     if(!row.fields.includes('weeklyValue'))assert.doesNotMatch(help,/7d 同期|周额度消耗比例/);
     if(!row.fields.includes('fiveHourValue'))assert.doesNotMatch(help,/5h|容量倍率/);
     if(index<displayCases.length-1)await entry.click();
   }
   await page.evaluate(async original=>{Object.assign(quotaFixture.rows[0],original);quotaFixture.records=[];await loadDashboard();},originalQuota);
   if(!wasExpanded)await codex.locator('.quota-title').click();
   assert.equal(await agy.locator('.provider-mark').getAttribute('data-provider'),'antigravity');
   const resetCheck=await page.evaluate(()=>{
     const now=1800000000,window=quotaFixture.rows[0].windows[0],original=JSON.stringify(window),node=document.querySelector('[data-quota="codex:c"] [data-reset-at]');
     node.dataset.resetAt=String(now-1);
     updateResetDisplays(now);const first=node.textContent;updateResetDisplays(now+86400);const later=node.textContent;
     node.dataset.resetAt='0';updateResetDisplays(now);const missing=node.textContent;
     node.dataset.resetAt=String(now+3600);updateResetDisplays(now);const refreshed=node.textContent;
     node.dataset.resetAt=String(window.resetsAt);updateResetDisplays();
     return {first,later,missing,refreshed,unchanged:JSON.stringify(window)===original};
   });
   assert.equal(resetCheck.first,'确认重置中');assert.equal(resetCheck.later,'确认重置中');assert.equal(resetCheck.missing,'重置时间未知');assert.match(resetCheck.refreshed,/^1 小时后重置 · \d{2}\/\d{2} \d{2}:\d{2}$/);assert(resetCheck.unchanged);
   const fullWindowCheck=await page.evaluate(()=>{
     const clock=Date.now,now=Math.floor(clock()/1000),q=state.dashboard.quotas.find(q=>q.provider==='codex'),saved=q.windows;
     const nodes=()=>[...document.querySelectorAll('[data-quota="codex:c"] [data-reset-at]')];
     try {
       Date.now=()=>now*1000;
       q.windows=[{name:'5h',usedPercent:0,windowMinutes:300,resetsAt:now+7200},{name:'7d',usedPercent:20,windowMinutes:10080,resetsAt:now+172800}];
       const snapshot=JSON.stringify(q.windows);renderAgent();const initial=nodes().map(n=>n.textContent);
       Date.now=()=>(now+3600)*1000;quotaTimers.find(t=>t.ms===30000).fn();const later=nodes().map(n=>n.textContent);
       const unchanged=JSON.stringify(q.windows)===snapshot;
       q.windows[0].usedPercent=0.001;renderAgent();const activated=nodes()[0].textContent;
       q.windows[0].usedPercent=0;q.windows[1].usedPercent=0;renderAgent();const full=nodes().map(n=>n.textContent);
       q.windows[0].usedPercent=null;renderAgent();quotaTimers.find(t=>t.ms===30000).fn();const missing=nodes()[0].textContent;
       Date.now=()=>(now+7200)*1000;quotaTimers.find(t=>t.ms===30000).fn();const expired=nodes()[0].textContent;
       q.windows[0].resetsAt=null;renderAgent();const unknown=nodes()[0].textContent;quotaTimers.find(t=>t.ms===30000).fn();const unknownLater=nodes()[0].textContent;
       q.windows[0].usedPercent=0;q.windows[1].resetsAt=null;renderAgent();const fullUnknown=nodes().map(n=>n.textContent);
       quotaTimers.find(t=>t.ms===30000).fn();const fullUnknownLater=nodes().map(n=>n.textContent);
       q.windows[0].resetsAt=now+9000;renderAgent();const refreshed=nodes()[0].textContent;
       return {initial,later,unchanged,activated,full,missing,expired,unknown,unknownLater,fullUnknown,fullUnknownLater,refreshed};
     } finally {Date.now=clock;q.windows=saved;renderAgent();}
   });
   assert.match(fullWindowCheck.initial[0],/^2 小时后重置 · /);assert.match(fullWindowCheck.initial[1],/^2 天后重置 · /);
   assert.match(fullWindowCheck.later[0],/^1 小时后重置 · /);assert.equal(fullWindowCheck.initial[0].split(' · ')[1],fullWindowCheck.later[0].split(' · ')[1]);
   assert.match(fullWindowCheck.later[1],/^1 天 23 小时后重置 · /);assert(fullWindowCheck.unchanged);
   assert.match(fullWindowCheck.activated,/^1 小时后重置 · /);assert.match(fullWindowCheck.missing,/^1 小时后重置 · /);
   assert.match(fullWindowCheck.full[0],/^1 小时后重置 · /);assert.match(fullWindowCheck.full[1],/^1 天 23 小时后重置 · /);
   assert.equal(fullWindowCheck.expired,'确认重置中');assert.equal(fullWindowCheck.unknown,'重置时间未知');assert.equal(fullWindowCheck.unknownLater,'重置时间未知');assert.match(fullWindowCheck.refreshed,/^30 分后重置 · /);
   assert.deepEqual(fullWindowCheck.fullUnknown,['5h','7d']);assert.deepEqual(fullWindowCheck.fullUnknownLater,['5h','7d']);
   const icons=await page.evaluate(async()=>{
     const holder=document.createElement('div');holder.innerHTML=['codex','claude','antigravity','agy','deepseek','custom'].map(provider=>providerMark(AieyesUI.canonicalProvider(provider))).join('');document.body.append(holder);
     try { await Promise.all([...holder.querySelectorAll('img')].map(img=>img.decode()));return {images:holder.querySelectorAll('img').length,fallback:holder.querySelectorAll('svg').length}; } finally {holder.remove();}
   });assert.deepEqual(icons,{images:5,fallback:1});
   await agy.locator('.quota-title').click();assert.equal(await agy.locator('.quota-reset:visible').count(),0,'Collapsed accounts hide reset details');assert.equal(await agy.locator('.resource-bar:visible').count(),2,'Collapsed accounts retain two key windows');await agy.locator('.quota-title').click();assert.equal(await agy.locator('.resource-bar:visible').count(),6,'Expanding restores all model groups');
   await page.locator('#order-quotas').click();await page.locator('[data-order="antigravity:a"] [data-direction="-1"]').click();
   // Button reordering animates both rows. Native drag computes its target
   // before dragstart cancels those animations, so wait for stable positions.
   await page.evaluate(async()=>{await document.fonts.ready;await Promise.all(document.getAnimations().filter(a=>a.effect?.getTiming().iterations!==Infinity).map(a=>a.finished.catch(()=>{})));});
   // Check each native drop before proceeding to the save/retry flow.
   await page.locator('[data-order="codex:c"]').dragTo(page.locator('[data-order="antigravity:a"]'),{steps:10});
   await page.waitForFunction(()=>document.querySelector('[data-order]').dataset.order==='codex:c');await page.evaluate(()=>Promise.all(document.getAnimations().filter(a=>a.effect?.getTiming().iterations!==Infinity).map(a=>a.finished.catch(()=>{}))));
   await page.locator('[data-order="antigravity:a"]').dragTo(page.locator('[data-order="codex:c"]'),{steps:10});
   await page.waitForFunction(()=>document.querySelector('[data-order]').dataset.order==='antigravity:a');
   await page.evaluate(()=>quotaFixture.fail=true);await page.locator('#quota-order-save').click();await page.waitForFunction(()=>document.querySelector('#quota-tool-error').textContent.includes('保存失败'));
   assert(await page.locator('#quota-dialog').evaluate(e=>e.open));await page.evaluate(()=>quotaFixture.fail=false);await page.locator('#quota-order-save').click();await page.waitForFunction(()=>!document.querySelector('#quota-dialog').open);
   assert.equal(await page.locator('[data-quota]').first().getAttribute('data-quota'),'antigravity:a');
   if(!await page.locator('[data-quota="codex:c"]:visible .quota-disclosure').evaluate(el=>el.open))await page.locator('[data-quota="codex:c"]:visible .quota-disclosure summary').click();
   await page.locator('[data-estimate="codex:c"]:visible').click();assert(await page.locator('#estimate-start').isDisabled());await page.locator('#estimate-confirm').check();
   await page.locator('[name=estimate-source]').uncheck();assert(await page.locator('#estimate-start').isDisabled(),'No sources disables sampling');
   await page.locator('[name=estimate-source]').check();await page.locator('#estimate-start').click();await page.locator('#estimate-stop').waitFor();assert.equal(await page.locator('#quota-dialog').getByText('$20.00 USD',{exact:true}).count(),1);
   await page.locator('#quota-dialog details summary').click();await page.screenshot({path:path.join(output,surface+'-estimate.png')});
   await page.keyboard.press('Escape');await page.waitForFunction(()=>!document.querySelector('#quota-dialog').open);if(surface==='floating.html')assert.equal(await page.evaluate(()=>quotaFixture.panelOpen),true,'Escape does not close underlying panel');
   await page.locator('[data-estimate="codex:c"]:visible').click();
   await page.evaluate(async()=>{quotaFixture.records[0].status='pending';quotaFixture.records[0].reason='检测到提前重置';await loadDashboard();});
   await page.locator('#estimate-restart').waitFor();assert(await page.locator('#estimate-restart').isDisabled());await page.locator('#estimate-confirm').check();await page.locator('#estimate-restart').click();await page.locator('#estimate-stop').waitFor();
   await page.locator('#estimate-stop').click();await page.locator('#estimate-start').waitFor();assert.equal(await page.locator('#quota-dialog').getByText('已结束',{exact:true}).count(),1);
   await page.locator('#quota-close').click();
   await page.evaluate(()=>openProviderEstimates('codex'));
   assert.equal(await page.locator('#manual-sampling').evaluate(el=>el.open),false);
   assert.equal(await page.locator('[data-valuation-record]').count(),0);
   assert.match(await page.locator('#quota-dialog').innerText(),/自动估值5h\/7d限额和credits的价值/);
   const beforeConsent=await page.evaluate(()=>quotaFixture.calls.filter(c=>c.method==='autoEstimates.configure').length);
   await page.locator('[data-auto-save="c"]').click();await page.locator('#auto-consent [data-cancel]').click();
   assert.equal(await page.locator('[data-auto-save="c"]').isChecked(),false);
   assert.equal(await page.evaluate(()=>quotaFixture.calls.filter(c=>c.method==='autoEstimates.configure').length),beforeConsent);
   await page.evaluate(()=>quotaFixture.failAutomatic=true);
   await page.locator('[data-auto-save="c"]').click();await page.locator('#auto-consent [data-confirm]').click();
   await page.locator('[data-auto-error]:visible').waitFor();
   assert.equal(await page.locator('[data-auto-save="c"]').isChecked(),false,'Failed opt-in retains the saved switch');
   await page.evaluate(()=>quotaFixture.failAutomatic=false);
   await page.locator('[data-auto-save="c"]').click();await page.locator('#auto-consent [data-confirm]').click();
   await page.waitForFunction(()=>state.settings.sources.find(s=>s.id==='c').autoValuation?.enabled===true&&!document.querySelector('#quota-dialog').dataset.busy);
   assert(await page.locator('[data-auto-save="c"]').isChecked());
   assert.match(await page.locator('[data-auto-status="c"]').textContent(),/等待有效样本/);
   await page.locator('#quota-close').click();
   if(surface==='index.html'){
     await page.evaluate(()=>{state.page='settings';state.settingsTab='sources';render();});
     const automatic=page.locator('[data-auto-machine-status="local"][data-auto-provider="codex"]');
     assert.match(await automatic.textContent(),/已开启 1\/1/,'Machine rows expose valuation status');
     assert.equal(await page.locator('[data-machine-estimates]').count(),0,'No per-machine settings button');
     await page.locator('[data-machine-directories="codex"]').first().click();
     assert.equal(await page.locator('[data-auto-source], [data-machine-estimates]').count(),0,'Directory settings contain no automatic valuation entry');
     await page.locator('#directories-back').click();
     await page.locator('[data-settings-agent=claude]').click();
     assert.match(await page.locator('[data-auto-machine-status="local"][data-auto-provider="claude"]').textContent(),/未配置目录/);
     await page.locator('[data-agent-estimates="claude"]').click();
     assert.match(await page.locator('#quota-dialog').innerText(),/自动估值5h\/7d限额的价值/);
     assert(!await page.locator('#quota-dialog').innerText().then(text=>text.includes('credits')));
     await page.locator('#quota-close').click();
     await page.screenshot({path:path.join(output,'source-automatic-settings.png'),fullPage:true});
     await page.locator('[data-settings-agent=codex]').click();await page.locator('[data-agent-estimates="codex"]').click();
   }else await page.evaluate(()=>openProviderEstimates('codex'));
   await page.locator('[data-auto-save="c"]').click();
   await page.waitForFunction(()=>!state.settings.sources.find(s=>s.id==='c').autoValuation?.enabled&&!document.querySelector('#quota-dialog').dataset.busy);
   assert.equal(await page.locator('#auto-consent').count(),0,'Disabling saves directly');
   await page.locator('#quota-close').click();
   if(surface==='index.html')assert.match(await page.locator('[data-auto-machine-status="local"][data-auto-provider="codex"]').textContent(),/已关闭/,'Machine status updates after saving');
   await page.evaluate(()=>{state.page='agent';render();});
   for(const colorScheme of ['light','dark']){await page.emulateMedia({colorScheme});await page.screenshot({path:path.join(output,surface+'-'+colorScheme+'.png')});}
   assert.deepEqual(errors,[]);await page.close();
  }
  console.log('Quota UI: account-specific valuation fields, live plan/window changes, agy compact/expanded, ordering retry, manual start/stop, live reset confirmation/restart, source validation, history, focus, panel Escape and shared-home automatic consent/settings sync passed on both surfaces');
 }finally{await browser?.close();await new Promise(r=>server.close(r));}
})().catch(e=>{console.error(e);process.exitCode=1;});
