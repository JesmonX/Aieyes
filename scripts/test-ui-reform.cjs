const {discardEditor,commitSettings}=require('./ui-test-helpers.cjs');
// 10-06 behavior and visual matrix. All data and RPC responses are synthetic.
const assert=require('node:assert/strict');
const fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require(process.env.AIEYES_PLAYWRIGHT||'../apps/desktop/node_modules/playwright');
const root=path.resolve(__dirname,'../apps/desktop/web');
const output=path.resolve(__dirname,'../.local/ui-reform-2026-10-06/desktop');
function fixture(){
 const stamp=1791260400;
 const tokens={input:50000,output:20000,cacheRead:25000,cacheWrite:5000};
 const usage={tokens,total:100000,events:20,pricedTokens:100000,cost:1.23};
 const source={id:'s1',name:'本机 Codex',provider:'codex',accountId:'a1',path:'/fixture',enabled:true,hostId:'h1'};
 const account={id:'a1',name:'个人订阅',provider:'codex',quotaEnabled:true};
 const host={id:'h1',name:'训练服务器',target:'fixture.invalid',enabled:true,metrics:['cpu'],details:[],devices:[]};
 const quota={provider:'codex',accountId:'a1',name:account.name,sourceId:'s1',updatedAt:stamp,origin:'live',plan:'pro',windows:[{id:'5h',name:'5h',usedPercent:25,windowMinutes:300,resetsAt:stamp+18000},{id:'7d',name:'7d',usedPercent:45,windowMinutes:10080,resetsAt:stamp+604800}]};
 window.reform={stamp,usage,source,account,host,quota,scenario:'single',failures:[],saved:[],calls:[],waitScan:false,releaseScan:null};
 window.setInterval=()=>1;
 const info=()=>({platform:'windows',material:'mica',mode:'auto',effectiveMode:'floating',sessions:[],summary:'暂无活跃会话',activeCount:0,panelOpen:true,panelPinned:false,hidden:false});
 const settings=()=>({sources:reform.scenario==='empty'?[]:[structuredClone(source)],accounts:reform.scenario==='empty'?[]:[structuredClone(account)],hosts:[structuredClone(host)],modelMappings:Object.fromEntries(Array.from({length:30},(_,i)=>['model-'+i,'provider/model-'+i])),proxy:{mode:'system',url:''},refreshSeconds:300,serverRefreshSeconds:10});
 window.reformListeners={};window.reformEmit=(name,payload)=>Promise.all((window.reformListeners[name]??[]).map(fn=>fn({payload})));
 window.__TAURI__={event:{async listen(name,fn){(window.reformListeners[name]??=[]).push(fn);return ()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){},async toggleMaximize(){},async minimize(){},async close(){}})},core:{async invoke(command,args){
  if(command==='desktop_info')return info();if(command==='desktop_panel')return {...info(),panelOpen:args.open};if(command!=='engine_call')return {};
  const r=reform;r.calls.push({method:args.method,params:structuredClone(args.params)});
  switch(args.method){
   case 'hello':return {version:'fixture'};
   case 'settings.get':{const current=settings(),saved=r.saved.at(-1);return saved?{...structuredClone(saved),sources:current.sources,accounts:current.accounts,hosts:current.hosts}:current;}
   case 'settings.save':r.saved.push(args.params);return {};
   case 'sources.scan':if(r.waitScan)await new Promise(resolve=>r.releaseScan=resolve);return r.failures.filter(row=>!args.params.sourceId||row.id===args.params.sourceId);
   case 'quotas.refresh':return [quota];
   case 'hosts.sample':return [{id:'h1',name:host.name,sample:{timestamp:stamp,load:[0.1],errors:{},cpu:[{id:'cpu',utilization:40}]}}];
   case 'prices.list':return [{id:'model',name:'模型',input:.000001,output:.000002}];
   case 'dashboard':{
    const empty=r.scenario==='empty'||args.params.model==='missing';
    const summary=empty?{tokens:{input:0,output:0,cacheRead:0,cacheWrite:0},total:0,cost:0,events:0,pricedTokens:0}:usage;
    const days=Array.from({length:7},(_,i)=>({...summary,key:'2026-10-'+String(i+1).padStart(2,'0')}));
    let quotas=empty?[]:[structuredClone(quota)];
    if(r.scenario==='multi')quotas=Array.from({length:5},(_,i)=>({...quota,accountId:'a'+i,name:'账户 '+(i+1)}));
    if(r.scenario==='long')quotas[0].name='团队账户与训练服务器的长名称示例'.repeat(4);
    return {generatedAt:stamp,summary,quotas,models:empty?[]:[{...summary,key:'model'}],modelOptions:['model','missing'],dayModels:empty?[]:days.map(d=>({day:d.key,model:'model',usage:summary})),trendDays:days,heatmap:days,sources:[],pricingGaps:[]};
   }
   default:return {};
  }
 }}};
}
(async()=>{
 const server=http.createServer((req,res)=>{const name=path.basename(new URL(req.url,'http://localhost').pathname)||'index.html';const file=path.join(root,name);if(!fs.existsSync(file)){res.writeHead(404);res.end();return;}res.setHeader('content-type',({'.html':'text/html','.css':'text/css','.js':'application/javascript'})[path.extname(file)]||'application/octet-stream');res.end(fs.readFileSync(file));});
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));fs.mkdirSync(output,{recursive:true});let browser;
 try{
  browser=await chromium.launch({headless:true});const errors=[];
  for(const surface of ['index.html','floating.html']){
   const panel=surface==='floating.html';const page=await browser.newPage({viewport:panel?{width:450,height:540}:{width:1120,height:800}});page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);await page.evaluate(()=>AieyesApp.ready);await page.waitForFunction(()=>!state.busy);
   const trigger=panel?'#panel-refresh':'#refresh',menu=panel?'#panel-menu':'#refresh-menu';
   await page.locator(trigger).focus();await page.keyboard.press('ArrowDown');assert.equal(await page.locator(menu+' [data-refresh]').count(),4);await page.keyboard.press('End');assert.equal(await page.locator(menu+' [data-refresh=hosts]').evaluate(el=>el===document.activeElement),true);await page.keyboard.press('Escape');assert.equal(await page.locator(trigger).evaluate(el=>el===document.activeElement),true);
   const initial=await page.locator('#activity').textContent();assert.match(initial,/记录/);
   await page.evaluate(()=>{reform.waitScan=true;window.pendingScan=scan();});assert.match(await page.locator('#activity').textContent(),/同步记录中/);await page.locator(trigger).click();assert.equal(await page.locator(menu+' [data-refresh=scan]').isDisabled(),true);await page.keyboard.press('Escape');
   await page.evaluate(async()=>{reform.waitScan=false;reform.releaseScan();await window.pendingScan;});assert.equal(await page.locator('#activity').textContent(),initial,'Success uses the dashboard timestamp, not completion time');
   await page.evaluate(async()=>{reform.failures=[{id:'s1',error:'来源一失败'},{id:'s2',error:'来源二失败'}];await scan();});assert.equal(await page.locator('.error-row').count(),2);assert.match(await page.locator('#activity').textContent(),/部分失败/);
   await page.evaluate(()=>{reform.failures=[{id:'s2',error:'来源二失败'}];});await page.locator('[data-retry=scan]').first().click();await page.waitForFunction(()=>!state.busy);assert.equal(await page.locator('.error-row').count(),1);assert.equal(await page.evaluate(()=>reform.calls.filter(c=>c.method==='sources.scan').at(-1).params.sourceId),'s1');
   await page.evaluate(async()=>{reform.failures=[];await scan();});assert.equal(await page.locator('.error-row').count(),0);
   // Shared RPC feedback must survive a filtered dashboard invalidation and retries.
   await page.evaluate(async()=>{
    await reformEmit('desktop:refresh-status',{key:'scan',busy:false,success:null,failures:[{id:'s1',error:'来源一失败'},{id:'s2',error:'来源二失败'}]});
    await reformEmit('desktop:data-changed','sources.scan');
   });
   assert.equal(await page.locator('.error-row').count(),2,'Filtered dashboard must not erase shared failures');
   await page.evaluate(async()=>{
    await reformEmit('desktop:refresh-status',{key:'scan',busy:false,itemId:'s1',success:Date.now(),failures:[]});
    await reformEmit('desktop:data-changed','sources.scan');
   });
   assert.equal(await page.locator('.error-row').count(),1,'Targeted success must retain other source failures');
   await page.evaluate(async()=>{
    await reformEmit('desktop:refresh-status',{key:'scan',busy:true});
    await reformEmit('desktop:data-changed','sources.scan');
   });
   assert.match(await page.locator('#activity').textContent(),/同步记录中/,'Data invalidation cannot complete a newer refresh');
   await page.evaluate(async()=>{
    await reformEmit('desktop:refresh-status',{key:'scan',busy:false,success:Date.now(),failures:[]});
    state.hosts=[{id:'h1',sample:{timestamp:reform.stamp}},{id:'h2',sample:{timestamp:reform.stamp}}];
    await reformEmit('desktop:refresh-status',{key:'hosts',busy:false,itemId:'h1',success:Date.now(),failures:[]});
    await reformEmit('desktop:hosts',[{id:'h1',sample:{timestamp:reform.stamp+60}}]);
   });
   assert.equal(await page.evaluate(()=>state.hosts.find(h=>h.id==='h1').sample.timestamp),1791260460,'Both windows receive host samples');
   assert.equal(await page.evaluate(()=>state.hosts.length),2,'Targeted host samples retain the other hosts');
   await page.evaluate(async()=>{state.model='missing';await loadDashboard();});assert.equal(await page.locator('.stat').count(),0);if(panel)await page.locator('.panel-filter-summary > summary').click();assert.equal(await page.getByRole('button',{name:'清除全部',exact:true}).count(),1);await page.getByRole('button',{name:'清除全部',exact:true}).click();await page.waitForFunction(()=>state.dashboard.summary.total>0);
   if(panel){
    assert.equal(await page.locator('[data-agent-detail=panel-trend]').evaluate(el=>el.open),false);
    assert.equal(await page.locator('.quota-disclosure').first().evaluate(el=>el.open),true);
    for(const row of await page.locator('.quota-windows .quota-window > .between').all()){
     assert.equal(await row.isVisible(),true,'Single account shows all window names and remaining percentages');
     assert.match(await row.textContent(),/剩余 \d/);
    }
   }
   else{
    await page.keyboard.press('Control+,');assert.equal(await page.locator('#general-form').count(),0);await page.locator('[data-settings-tab=general]').click();assert.equal(await page.locator('#settings-save-all').count(),1);assert.equal(await page.locator('#connection-form').count(),0);
    await page.locator('#field-refreshSeconds').fill('180');await page.locator('#settings-save-all').click();await page.waitForFunction(()=>!state.busy);assert.match(await page.locator('.settings-draft-label').textContent(),/已保存/);assert.equal(await page.evaluate(()=>state.settings.refreshSeconds),180);assert.equal(await page.evaluate(()=>reform.saved.at(-1).refreshSeconds),180);
    await page.locator('[data-settings-tab=hosts]').click();await page.locator('[data-remove=h1]').click();assert.match(await page.locator('#editor-fields').textContent(),/本机 Codex/);assert.equal(await page.locator('#cancel-editor').evaluate(el=>el===document.activeElement),true);await page.keyboard.press('Escape');assert.equal(await page.evaluate(()=>state.settings.hosts.length),1);
    await page.keyboard.press('Control+Shift+D');
   }
   for(const scenario of ['empty','single','multi','long','failure'])for(const theme of ['light','dark']){
    await page.emulateMedia({colorScheme:theme});await page.evaluate(async scenario=>{notify('');reform.scenario=scenario;state.page='agent';state.settings=await api('settings.get');state.provider=state.accountKey=state.sourceId=state.model='';await loadDashboard();if(scenario==='failure'){reform.failures=[{id:'s1',error:'连接失败：模拟 SSH 与日志不可读取'}];await scan();}else{reform.failures=[];await scan();}},scenario);
    if(scenario==='empty'){assert.equal(await page.locator('.stat').count(),0);assert.equal(await page.locator('.filters').count(),1);assert.equal(await page.locator('.onboarding').count(),1);}
    if(panel&&scenario==='single'){
     await page.screenshot({animations:'disabled',path:path.join(output,'panel-small-layout.png')});
     const footer=await page.locator('.panel-foot').boundingBox();
     for(const value of await page.locator('.stat-value').all()){
      const box=await value.boundingBox();assert.ok(box.y+box.height<=footer.y,`Today values fit above the footer at 450×540: ${JSON.stringify({box,footer})}`);
     }
    }
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true,`${surface}/${scenario}/${theme} horizontal overflow`);
    await page.screenshot({animations:'disabled',path:path.join(output,`${panel?'panel':'detail'}-${scenario}-${theme}.png`)});
   }
   if(!panel){
    await page.evaluate(async()=>{reform.scenario='single';state.settings=await api('settings.get');await loadDashboard();});await page.setViewportSize({width:640,height:440});
    for(const theme of ['light','dark']){await page.emulateMedia({colorScheme:theme});await page.screenshot({animations:'disabled',path:path.join(output,`minimum-detail-${theme}.png`)});}
    await page.keyboard.press('Control+,');await page.locator('[data-settings-tab=prices]').click();assert.equal(await page.locator('[data-unmap]').count(),30);await page.screenshot({animations:'disabled',path:path.join(output,'mappings-30-minimum.png')});
    await page.locator('[data-settings-tab=hosts]').click();await page.locator('[data-edit=h1]').click();await page.screenshot({animations:'disabled',path:path.join(output,'editor-minimum.png')});await page.keyboard.press('Escape');
   }
   await page.close();
  }
  // Real deviceScaleFactor contexts exercise rendering at 150% and 200% DPI.
  for(const scale of [1.5,2])for(const theme of ['light','dark']){
   const context=await browser.newContext({deviceScaleFactor:scale,colorScheme:theme,viewport:{width:1120,height:800}});const page=await context.newPage();page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}`);await page.evaluate(()=>AieyesApp.ready);await page.screenshot({animations:'disabled',path:path.join(output,`detail-dpi-${scale}-${theme}.png`)});await context.close();
  }
  assert.deepEqual(errors,[]);console.log('10-06 refresh states, data time, targeted retries, empty/filter recovery, destructive focus, saves, shortcuts and 28 screenshot checks passed');
 }finally{await browser?.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
