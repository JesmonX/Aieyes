const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
function fixture(){
  window.setInterval=()=>0;
  const at=Date.now()/1000,groups=['Gemini Models','Claude and GPT Models','Future Models'];
  const q={provider:'antigravity',accountId:'legacy-shared',sourceId:'agy-source',name:'Antigravity',plan:'ultra',updatedAt:at,origin:'live',windows:groups.flatMap(group=>[300,10080].map(minutes=>({id:group+':'+minutes,name:group+' · '+(minutes===300?'5h':'7d'),groupId:group,groupName:group,windowMinutes:minutes,usedPercent:20,resetsAt:at+minutes*60})))};
  const codex={...q,provider:'codex',accountId:'c',sourceId:'c',name:'Codex',plan:'pro',windows:[],credits:{hasCredits:true,unlimited:false,balance:'1000'}};
  const record=(group,value)=>({id:group,accountKey:'antigravity:legacy-shared',groupId:group,windowId:group+':300',windowName:group+' · 5h',valuationMode:'fiveHour',status:'completed',sourceIds:['agy-source'],sourceNames:['CLI'],startedAt:at-100,checkpointAt:at,consumedPercent:10,cost:1,totalTokens:100,pricedTokens:100,fiveHourValue:value,weeklyValue:value*6,weeklyDirectValue:value*6,weeklyRatioValue:value*5,calculationNote:'分组估值',prices:[]});
  const settings={version:2,accountAliases:{'agy:shared':'antigravity:legacy-shared'},accounts:[q,codex].map(q=>({id:q.accountId,name:q.name,provider:q.provider,quotaEnabled:true})),sources:[q,codex].map(q=>({id:q.sourceId,provider:q.provider,accountId:q.accountId,name:q.name,path:'/fixture',enabled:true})),hosts:[],modelMappings:{},proxy:{mode:'direct',url:''},refreshSeconds:300,serverRefreshSeconds:10};
  const empty={tokens:{input:0,output:0,cacheRead:0,cacheWrite:0},total:0,cost:0,pricedTokens:0,events:0};
  window.agyFixture={settings,q,records:[record(groups[0],10),record(groups[1],100)],calls:[]};
  localStorage.setItem('aieyes.panel.accounts.v2',JSON.stringify({version:2,providers:{agy:{mode:'custom',keys:['agy:shared']}}}));
  localStorage.setItem('quota.expanded.v2.panel.agy:shared','false');
  localStorage.setItem('aieyes.panel.view',JSON.stringify({provider:'agy',accountKey:'agy:shared'}));
  window.__TAURI__={event:{async listen(){return ()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return ()=>{};}})},core:{async invoke(command,args){
    if(command==='desktop_info')return {platform:'windows',mode:'floating',effectiveMode:'floating',panelOpen:true,sessions:[]};
    if(command!=='engine_call')return {};
    const f=agyFixture,{method,params}=args;f.calls.push({method,params});
    if(method==='hello')return {version:'test'};if(method==='settings.get')return settings;
    if(method==='dashboard')return {generatedAt:at,summary:empty,quotas:[q,codex],quotaOrder:['antigravity:legacy-shared','codex:c'],quotaEstimates:f.records,creditEstimates:[{...record('',1),id:'credit',accountKey:'codex:c',valuePer1000:200}],modelOptions:[],models:[],dayModels:[],trendDays:[],heatmap:[],sources:[],pricingGaps:[]};
    if(method==='quotaEstimates.start'){const group=q.windows.find(w=>w.id===params.windowId).groupId;f.records.unshift({...record(group,15),id:'active',status:'active'});return f.records[0];}
    if(method==='quotaEstimates.stop'){f.records.find(r=>r.id===params.id).status='completed';return {};}
    return [];
  }}};
}
(async()=>{
 const root=path.resolve(__dirname,'../apps/desktop/web'),out=path.resolve(__dirname,'../.local/compact-antigravity/web');fs.mkdirSync(out,{recursive:true});
 const server=http.createServer((req,res)=>{if(require('./ui-test-helpers.cjs').serveFont(req,res))return;const file=path.join(root,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
 await new Promise(r=>server.listen(0,'127.0.0.1',r));const browser=await chromium.launch({headless:true});
 try{for(const surface of ['index.html','floating.html']){
  const panel=surface==='floating.html',page=await browser.newPage({viewport:{width:panel?450:1120,height:850}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);await page.evaluate(()=>AieyesApp.ready);
  const card=page.locator('[data-quota="antigravity:legacy-shared"]');await card.waitFor();
  assert.equal(await card.locator('.subscription-badge').textContent(),'Ultra');
  const pref=await page.evaluate(()=>JSON.parse(localStorage.getItem('aieyes.panel.accounts.v2')));assert.deepEqual(pref.providers.antigravity.keys,['antigravity:legacy-shared']);assert(!pref.providers.agy);
  if(panel){assert.equal(await page.evaluate(()=>state.accountKey),'antigravity:legacy-shared');assert.equal(await page.evaluate(()=>state.provider),'antigravity');assert.equal(await card.locator('details').evaluate(e=>e.open),false);await card.locator('.quota-title').click();}
  const entries=card.locator('[data-estimate]');assert.equal(await entries.count(),3);
  assert.equal(await entries.nth(0).innerText(),'5h ≈ $10.00 7d ≈ $50.00/$60.00');assert.equal(await entries.nth(1).innerText(),'5h ≈ $100.00 7d ≈ $500.00/$600.00');
  await page.evaluate(()=>{agyFixture.records.push({...agyFixture.records[0],id:'legacy',groupId:null,valuationMode:'weekly',fiveHourValue:null,weeklyRatioValue:null});state.dashboard.quotaEstimates=agyFixture.records;render();});
  await entries.nth(1).click();assert.equal(await page.locator('#estimate-window').inputValue(),'Claude and GPT Models:300');assert.equal(await page.locator('#quota-dialog .estimate-result').count(),2);
  await page.evaluate(async()=>{agyFixture.records[1].checkpointAt++;await loadDashboard();});assert.equal(await page.locator('#estimate-window').inputValue(),'Claude and GPT Models:300');
  await page.locator('#estimate-confirm').check();await page.locator('#estimate-start').click();await page.locator('#estimate-stop').waitFor();
  assert.equal(await page.evaluate(()=>agyFixture.calls.filter(c=>c.method==='quotaEstimates.start').at(-1).params.windowId),'Claude and GPT Models:300');
  await page.locator('#estimate-stop').click();await page.locator('#estimate-start').waitFor();await page.locator('#quota-close').click();
  await entries.nth(2).click();assert.equal(await page.locator('#estimate-start').count(),0);assert.match(await page.locator('#quota-dialog').innerText(),/映射/);await page.locator('#quota-close').click();
  const credit=page.locator('[data-quota="codex:c"]');assert.equal(await credit.locator('[data-credit-estimate]').innerText(),'credits估值（当前1000credits≈$200.00）');
  for(const theme of ['light','dark'])for(const width of panel?[360,420,450]:[640,1120]){
    await page.emulateMedia({colorScheme:theme});await page.setViewportSize({width,height:850});
    await page.evaluate(()=>{for(const e of agyFixture.records){e.fiveHourValue=987654;e.weeklyRatioValue=987654321;e.weeklyValue=987654321012;}state.dashboard.quotaEstimates=agyFixture.records;render();});
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    assert(await card.evaluate(e=>e.getBoundingClientRect().right<=innerWidth),`${surface} ${width}: card extends beyond viewport`);
    if(panel)assert(await page.locator('.panel-body').evaluate(e=>e.scrollWidth<=e.clientWidth+1),'Panel content must fit without clipping');
    for(const entry of await entries.all())assert(await entry.locator('span').evaluate(e=>e.scrollWidth<=e.clientWidth+1),`${surface} ${width}: ${await entry.innerText()}`);
    await page.evaluate(()=>{document.querySelector('.panel-body')?.scrollTo(0,0);window.scrollTo(0,0);});
    await page.mouse.move(2,2);
    await page.screenshot({path:path.join(out,`${surface}-${width}-${theme}.png`),fullPage:true});
  }
  assert.deepEqual(errors,[]);await page.close();
 }console.log('Antigravity UI: provider preference migration, independent group values, sampling selection/refresh, unsupported groups, credits and large amounts at 360–1120px passed');}
 finally{await browser.close();await new Promise(r=>server.close(r));}
})().catch(e=>{console.error(e);process.exitCode=1;});
