const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
function fixture(){
 const now=Date.now()/1000,empty={tokens:{input:0,output:0,cacheRead:0,cacheWrite:0},total:0,cost:0,pricedTokens:0,events:0};
 const accounts=[{id:'a',provider:'codex',name:'订阅账户 · 较长的账户名称测试',quotaEnabled:true},{id:'new',provider:'agy',name:'Agy',quotaEnabled:true}];
 const base={accountKey:'codex:a',windowId:'weekly',windowName:'7d',sourceIds:['s'],sourceNames:['本机'],status:'completed',reason:'',startedAt:now-1000,checkpointAt:now-800,endedAt:now-800,consumedPercent:13,cost:6.7795,totalTokens:100,pricedTokens:100,calculationNote:'手动采样估值',prices:[]};
 const old={...base,id:'old',weeklyValue:52.15};
 const broken={...base,id:'broken',valuationMode:'fiveHour',startedAt:now-700,checkpointAt:now-500,fiveHourValue:null,weeklyValue:null,calculationStatus:'boundary',calculationNote:'存在跨采样边界的累计用量'};
 const credit={...base,id:'credit',kind:'credits',startedAt:now-700,consumedCredits:24.93366,valuePer1000:null,calculationStatus:'boundary',calculationNote:'存在跨采样边界的累计用量'};
 const q={provider:'codex',accountId:'a',sourceId:'s',name:accounts[0].name,plan:'Plus',updatedAt:now,origin:'live',windows:[{id:'5h',name:'5h',windowMinutes:300,usedPercent:96,resetsAt:now+2000},{id:'7d',name:'7d',windowMinutes:10080,usedPercent:15,resetsAt:now+8000}],credits:{balance:'1823.716559',hasCredits:true,unlimited:false}};
 window.samplingFixture={settings:{version:2,accounts,sources:[{id:'s',provider:'codex',accountId:'a',name:'本机',path:'/fixture',enabled:true},{id:'agy',provider:'agy',accountId:'new',name:'Agy',enabled:true}],hosts:[],modelMappings:{},proxy:{mode:'direct',url:''},refreshSeconds:300,serverRefreshSeconds:10},records:[broken,old],credits:[credit],calls:[],native:[]};
 window.setInterval=()=>1;
 window.__TAURI__={window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){}})},event:{async listen(){return()=>{};}},core:{async invoke(command,args){
  if(command==='desktop_info')return {page:'agent',platform:'windows',material:'opaque',effectiveMode:'floating',mode:'floating',sessions:[],summary:'暂无会话',panelOpen:true};
  if(command==='desktop_appearance'){samplingFixture.native.push(args);return {};}
  if(command!=='engine_call')return {};
  const {method,params}=args;samplingFixture.calls.push({method,params});
  if(method==='hello')return {version:'test'};
  if(method==='settings.get')return structuredClone(samplingFixture.settings);
  if(method==='settings.save'){samplingFixture.settings=structuredClone(params);return {};}
  if(method==='settings.patch'){samplingFixture.settings=structuredClone(params.settings);return structuredClone(samplingFixture.settings);}
  if(method==='dashboard')return {generatedAt:now,summary:empty,quotas:[q,{...q,provider:'agy',accountId:'new',sourceId:'agy',name:'Agy',credits:null,plan:null}],quotaEstimates:samplingFixture.records,creditEstimates:samplingFixture.credits,quotaOrder:['codex:a','agy:new'],modelOptions:[],models:[],dayModels:[],trendDays:[],heatmap:[],sources:[],pricingGaps:[]};
  if(method==='quotaEstimates.repair'){const fixed={...broken,id:'broken-repair-v2',originalEstimateId:'broken',repairedAt:now,fiveHourValue:6.7979,weeklyValue:43.5067,weeklyRatioValue:43.12,calculationStatus:'ready',calculationNote:'已修正'};samplingFixture.records.unshift(fixed);return fixed;}
  if(method==='creditEstimates.repair'){const fixed={...credit,id:'credit-repair-v2',originalEstimateId:'credit',valuePer1000:40.69585,calculationStatus:'ready',repairedAt:now};samplingFixture.credits.unshift(fixed);return fixed;}
  if(method==='network.test')return {state:'online',results:[]};
  return [];
 }}};
}
(async()=>{
 const root=path.resolve(__dirname,'../apps/desktop/web'),out=path.resolve(__dirname,'../.local/sampling-theme-ui');fs.mkdirSync(out,{recursive:true});
 const server=http.createServer((req,res)=>{if(require('./ui-test-helpers.cjs').serveFont(req,res))return;const file=path.join(root,req.url.split('?')[0]);if(!file.startsWith(root)||!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('Content-Type',({'.js':'text/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
 await new Promise(r=>server.listen(0,'127.0.0.1',r));const browser=await chromium.launch({headless:true});
 try{
 for(const surface of ['index.html','floating.html']){
  const page=await browser.newPage({viewport:{width:surface==='floating.html'?450:1120,height:760}}),errors=[];page.on('pageerror',e=>{errors.push(String(e));console.error('Page error:',e);});await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);await page.evaluate(()=>AieyesApp.ready);if(!await page.locator('[data-quota]').count())console.log(surface,await page.locator('body').innerText(),errors);
  const card=page.locator('[data-quota="codex:a"]');await card.waitFor();
  assert.match(await card.innerText(),/52.15/);assert.match(await card.innerText(),/用量边界待确认/);assert.doesNotMatch(await card.innerText(),/样本不足/);
  assert.equal(await card.locator('.subscription-badge').evaluate(e=>getComputedStyle(e).fontSize),'14px');
  assert(await page.locator('[data-quota="agy:new"]').isVisible());
  const selection=await page.evaluate(()=>{
   const settings=samplingFixture.settings;
   const stale={version:2,providers:{agy:{mode:'custom',keys:['agy:old']}}};
   const result=AieyesUI.panelAccounts(settings,[],stale);
   const hidden=AieyesUI.panelAccounts(settings,[],{version:2,providers:{agy:{mode:'custom',keys:[]}}});
   const untouched=JSON.stringify(stale);AieyesUI.panelAccounts({accounts:[]},[],stale);
   return {result,hidden,unchanged:untouched===JSON.stringify(stale)};
  });assert.deepEqual(selection.result.agy,['agy:new']);assert.deepEqual(selection.hidden.agy,[]);assert(selection.unchanged);
  await page.locator('[data-estimate="codex:a"]').click();await page.locator('[data-estimate-repair="broken"]').click();await page.waitForFunction(()=>!state.estimateBusy);await page.locator('#quota-close').click();await page.waitForFunction(()=>document.querySelector('[data-quota="codex:a"]').innerText.includes('6.80'));
  assert.match(await card.innerText(),/6.80/);assert.match(await card.innerText(),/已修正/);assert.equal(await page.evaluate(()=>samplingFixture.records.filter(e=>e.id==='broken').length),1);
  await page.locator('[data-credit-estimate="codex:a"]').click();await page.locator('[data-estimate-repair="credit"]').click();await page.waitForFunction(()=>!state.estimateBusy);await page.locator('#quota-close').click();await page.waitForFunction(()=>document.querySelector('[data-quota="codex:a"]').innerText.includes('40.70'));assert.match(await card.innerText(),/40.70/);
  for(const theme of ['light','dark'])for(const accent of ['indigo','blue','teal','purple']){
   await page.evaluate(({theme,accent})=>{state.settings.appearance={theme,accent};render();},{theme,accent});
   assert.equal(await page.locator('html').getAttribute('data-theme'),theme);assert.equal(await page.locator('html').getAttribute('data-accent'),accent);
   assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
   if(accent==='indigo'){await page.evaluate(async()=>{window.scrollTo(0,0);const body=document.querySelector('.panel-body');if(body)body.scrollTop=0;await Promise.all(document.getAnimations().filter(a=>a.effect?.getTiming().iterations!==Infinity).map(a=>a.finished.catch(()=>{})));});await page.screenshot({path:path.join(out,`${surface}-${theme}.png`)});}
  }
  await page.emulateMedia({colorScheme:'light'});assert.equal(await page.locator('html').getAttribute('data-theme'),'dark','Explicit dark remains dark on a light system');
  if(surface==='index.html'){
   await page.evaluate(()=>{samplingFixture.settings=structuredClone(state.settings);state.page='settings';state.settingsTab='general';render();});
   await page.locator('[name=appearanceTheme]').selectOption('light');
   await page.waitForFunction(()=>!state.generalSavePending&&!state.settingsSaving&&!state.settingsCommitting&&samplingFixture.settings.appearance?.theme==='light');
   await page.locator('[name=appearanceAccent]').selectOption('teal');
   await page.waitForFunction(()=>!state.generalSavePending&&!state.settingsSaving&&!state.settingsCommitting&&samplingFixture.settings.appearance?.accent==='teal');
   assert.deepEqual(await page.evaluate(()=>samplingFixture.settings.appearance),{theme:'light',accent:'teal'});assert.equal(await page.locator('html').getAttribute('data-theme'),'light');
   await page.screenshot({path:path.join(out,'settings-theme.png')});
  }
  assert.deepEqual(errors,[]);await page.close();
 }
 console.log('Sampling history, correction, agy selection, subscription size, eight themes and saved appearance passed.');
 }finally{await browser.close();await new Promise(r=>server.close(r));}
})().catch(e=>{console.error(e);process.exitCode=1;});
