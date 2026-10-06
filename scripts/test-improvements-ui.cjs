// New account/password, proxy, mapping, floating footer and 5h sampling flows.
// All credentials and IPC are synthetic; no external services are contacted.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
function fixture(){
  const now=Math.floor(Date.now()/1000),tokens={input:100,output:20,cacheRead:80,cacheWrite:0},summary={tokens,total:200,cost:1,pricedTokens:200,events:2};
  const settings={version:2,accounts:[{id:'account',provider:'custom',name:'测试账户',quotaEnabled:true}],sources:[{id:'source',provider:'custom',name:'本机',accountId:'account',path:'/fixture',enabled:true}],hosts:[{id:'host',name:'服务器',target:'fixture.invalid',identityFile:'',shell:'/bin/bash',preCommand:'',metrics:['cpu'],devices:[],details:[],enabled:true}],proxy:{mode:'system',url:''},proxyTestUrls:['https://fixture.invalid/a','https://fixture.invalid/b'],modelMappings:{},refreshSeconds:300,serverRefreshSeconds:10};
  const quota={provider:'custom',accountId:'account',sourceId:'source',name:'测试账户',origin:'live',updatedAt:now,windows:[{id:'five',name:'5h',usedPercent:30,windowMinutes:300,resetsAt:now+18000},{id:'week',name:'7d',usedPercent:35,windowMinutes:10080,resetsAt:now+604800}]};
  const network={testedAt:now,mode:'system',status:'ok',averageMs:73,sites:[{url:settings.proxyTestUrls[0],latencyMs:60,error:null},{url:settings.proxyTestUrls[1],latencyMs:86,error:null}]};
  window.feature={settings,quota,network,records:[],calls:[],listeners:{},timers:[],saveFails:false,delaySample:false,update:{currentVersion:'1.2.3',latestVersion:'1.2.4',phase:'available',message:'发现新版本',prompt:false,automatic:true},panelOpen:true};
  const f=window.feature;f.emit=(name,payload)=>{for(const fn of f.listeners[name]??[])fn({payload:structuredClone(payload)});};
  window.setInterval=(fn,ms)=>{f.timers.push({fn,ms});return f.timers.length;};window.clearInterval=()=>{};
  const info=()=>({platform:'windows',mode:'floating',effectiveMode:'floating',material:'opaque',sessions:[],summary:'暂无活跃会话',panelOpen:f.panelOpen,panelPinned:false,hidden:false});
  window.__TAURI__={event:{async listen(name,fn){(f.listeners[name]??=[]).push(fn);return ()=>{f.listeners[name]=f.listeners[name].filter(x=>x!==fn);};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return ()=>{};}})},core:{async invoke(command,args){
    f.calls.push({command,args:structuredClone(args)});
    if(command==='desktop_info')return info();if(command==='desktop_panel'){f.panelOpen=args.open;return info();}if(command==='desktop_panel_cursor_inside')return true;
    if(command==='updates_info'||command==='updates_panel_check')return structuredClone(f.update);
    if(command==='updates_later'){f.update.prompt=false;return structuredClone(f.update);}
    if(command!=='engine_call')return {};
    const {method,params}=args;
    if(method==='hello')return {version:'1.2.3'};if(method==='settings.get')return structuredClone(settings);
    if(method==='settings.save'){if(f.saveFails)throw new Error('模拟保存失败');Object.assign(settings,structuredClone(params));return {};}
    if(method==='hosts.credentials.save')return {passwordRef:'ssh-fixture-'+f.calls.length};if(method==='hosts.credentials.delete')return {};
    if(method==='hosts.discover')return {cpu:[{id:'cpu'}],errors:{}};
    if(method==='prices.list')return Array.from({length:50},(_,i)=>({id:'provider/model-'+i,name:'模型 '+i,input:0.00001,output:0.00003}));
    if(method==='network.status')return structuredClone(network);
    if(method==='network.test')return structuredClone({...network,mode:params.proxy?.mode??settings.proxy.mode,status:f.unstable?'unstable':'ok'});
    if(method==='quotas.refresh')return [quota];if(method==='sources.scan'||method==='hosts.sample')return [];
    if(method==='dashboard'){if(f.holdDashboard)await new Promise(resolve=>f.releaseDashboard=resolve);return {generatedAt:now,summary,quotas:[quota],quotaEstimates:structuredClone(f.records),quotaOrder:['custom:account'],pricingGaps:[],modelOptions:[],sources:[],models:[],trendDays:[],dayModels:[],heatmap:[]};}
    if(method==='prices.recalculate'){if(f.delayReprice)await new Promise(resolve=>f.releaseReprice=resolve);return {};}
    if(method==='quotaEstimates.start'){
      f.emit('operations:busy',{operationId:params.operationId,busy:true});
      f.emit('operations:progress',{operationId:params.operationId,stage:'同步来源 · 本机'});
      if(f.delaySample)await new Promise(resolve=>f.releaseSample=resolve);
      f.records=[{id:'sample',accountKey:'custom:account',valuationMode:'fiveHour',windowId:'five',windowName:'5h',sourceIds:['source'],sourceNames:['本机'],status:'active',reason:'',startedAt:now-60,checkpointAt:now,consumedPercent:10,cost:2,totalTokens:200,pricedTokens:200,fiveHourValue:20,weeklyDirectValue:100,weeklyRatioValue:110,capacity:{ratio:5.5,samples:6,weeklyPercent:12,updatedAt:now},calculationNote:'5h 采样估值 · 7d 同期样本较少',prices:[],segments:[]}];
      f.emit('operations:busy',{operationId:params.operationId,busy:false});return structuredClone(f.records[0]);
    }
    if(method==='quotaEstimates.stop'){f.records[0].status='completed';return structuredClone(f.records[0]);}
    return [];
  }}};
}
(async()=>{
  const root=path.resolve(__dirname,'../apps/desktop/web'),output=path.resolve(__dirname,'../.local/improvements/web');fs.mkdirSync(output,{recursive:true});
  const server=http.createServer((req,res)=>{const file=path.join(root,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404);res.end();return;}res.setHeader('content-type',({'.html':'text/html','.js':'application/javascript','.css':'text/css'})[path.extname(file)]||'application/octet-stream');res.end(fs.readFileSync(file));});
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));let browser;
  try{
    browser=await chromium.launch({headless:true});
    for(const surface of ['index.html','floating.html']){
      const page=await browser.newPage({viewport:{width:surface==='floating.html'?450:1120,height:760}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));
      await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy&&window.AieyesNetwork&&window.AieyesUpdates);
      if(surface==='index.html'){
        await page.evaluate(()=>{state.page='settings';state.settingsTab='general';renderSettings();});
        await page.locator('#field-appMode').selectOption('custom');await page.locator('#field-appHost').fill('127.0.0.1');await page.locator('#field-appPort').fill('7890');
        await page.locator('#app-proxy-test').click();await page.waitForFunction(()=>document.querySelector('#app-proxy-result').textContent==='代理 73 ms');
        const probe=await page.evaluate(()=>feature.calls.find(c=>c.args?.method==='network.test').args.params);assert.equal(probe.proxy.url,'http://127.0.0.1:7890');assert.deepEqual(probe.urls,['https://fixture.invalid/a','https://fixture.invalid/b']);
        await page.evaluate(()=>feature.unstable=true);await page.locator('#app-proxy-test').click();await page.waitForFunction(()=>document.querySelector('#app-proxy-result').textContent==='连接不稳定');
        const ordinarySaveStart=await page.evaluate(()=>{renderSettings();state.lastQuota=12345;return feature.calls.length;});
        await page.locator('#field-refreshSeconds').fill('600');await page.locator('#general-form button.primary').click();await page.waitForFunction(()=>feature.settings.refreshSeconds===600&&!state.busy&&!state.settingsSaving);
        assert.deepEqual(await page.evaluate(start=>feature.calls.slice(start).map(c=>c.args?.method).filter(Boolean),ordinarySaveStart),['settings.save']);assert.equal(await page.evaluate(()=>state.lastQuota),12345);
        await page.evaluate(async()=>{state.settingsTab='prices';state.prices=await api('prices.list');renderSettings();});await page.locator('#field-mappingModel').fill('日志别名');await page.locator('[data-map-price="provider/model-49"]').click();
        assert.equal(await page.locator('#field-mappingId').inputValue(),'provider/model-49');assert.equal(await page.locator('#field-mappingModel').inputValue(),'日志别名');assert(await page.locator('#field-mappingId').evaluate(el=>el===document.activeElement));
        assert.equal(await page.locator('#content form').first().getAttribute('id'),'mapping-form');assert.deepEqual(await page.evaluate(()=>feature.settings.modelMappings),{});
        await page.evaluate(()=>{renderSettings();document.querySelector('#content').scrollTop=0;});assert.equal(await page.locator('#reprice').textContent(),'保存并重算');assert(await page.locator('#reprice').evaluate(el=>el.getBoundingClientRect().bottom<window.innerHeight));
        const repriceStart=await page.evaluate(()=>{feature.delayReprice=true;return feature.calls.length;});await page.locator('#reprice').click();await page.waitForFunction(()=>feature.releaseReprice);assert(await page.locator('#reprice').isDisabled());
        await page.evaluate(()=>feature.releaseReprice());await page.waitForFunction(()=>!state.busy);assert.deepEqual(await page.evaluate(start=>feature.calls.slice(start).map(c=>c.args?.method).filter(Boolean),repriceStart),['settings.save','prices.recalculate','dashboard']);
        await page.evaluate(()=>feature.holdDashboard=true);await page.locator('#field-mappingModel').fill('日志别名');await page.locator('#field-mappingId').fill('provider/model-49');await page.locator('#mapping-form > button').click();await page.waitForFunction(()=>feature.releaseDashboard&&!state.busy&&!state.settingsSaving);
        assert.equal(await page.evaluate(()=>feature.settings.modelMappings['日志别名']),'provider/model-49');await page.evaluate(()=>{feature.holdDashboard=false;feature.releaseDashboard();});
        await page.screenshot({path:path.join(output,'index.html-settings-prices.png')});
        await page.evaluate(()=>{state.settingsTab='hosts';renderSettings();});await page.locator('[data-edit="host"]').click();await page.locator('#field-authMode').selectOption('password');await page.locator('#field-username').fill('fixture-user');await page.locator('#field-password').fill('fixture secret');
        await page.locator('#discover-devices').click();await page.waitForFunction(()=>feature.calls.some(c=>c.args?.method==='hosts.credentials.delete'));
        await page.evaluate(()=>feature.saveFails=true);await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>document.querySelector('#editor-error').textContent.includes('模拟保存失败'));
        assert.equal(await page.evaluate(()=>feature.settings.hosts[0].passwordRef),undefined);assert.equal(await page.evaluate(()=>feature.calls.filter(c=>c.args?.method==='hosts.credentials.delete').length),2);
        await page.evaluate(()=>feature.saveFails=false);await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.querySelector('#editor').open);
        const host=await page.evaluate(()=>feature.settings.hosts[0]);assert.equal(host.authMode,'password');assert.equal(host.username,'fixture-user');assert(host.passwordRef.startsWith('ssh-fixture-'));assert(!JSON.stringify(host).includes('fixture secret'));
        await page.evaluate(()=>{state.page='agent';render();feature.emit('operations:busy',{operationId:'other-window',busy:true});state.lastQuota=0;feature.timers.find(t=>t.ms===500).fn();});
        const before=await page.evaluate(()=>feature.calls.filter(c=>c.args?.method==='quotas.refresh').length);
        await page.evaluate(()=>{feature.timers.find(t=>t.ms===500).fn();});assert.equal(await page.evaluate(()=>feature.calls.filter(c=>c.args?.method==='quotas.refresh').length),before);
        await page.evaluate(()=>feature.emit('operations:busy',{operationId:'other-window',busy:false}));
      }else{
        await page.waitForFunction(()=>document.querySelector('#panel-latency').textContent==='系统 73 ms');assert(await page.locator('#panel-update .update-badge').isVisible());assert(!await page.locator('#panel-pin').isVisible());
        await page.locator('#panel-update').click();await page.waitForFunction(()=>feature.calls.some(c=>c.command==='updates_panel_check'));assert(await page.locator('#panel-update .update-badge').isVisible());
        await page.locator('#panel-more > summary').click();assert(await page.locator('#panel-pin').isVisible());await page.locator('#panel-more > summary').click();
      }
      const disclosure=page.locator('[data-quota="custom:account"] .quota-disclosure');if(!await disclosure.evaluate(el=>el.open))await disclosure.locator('summary').click();
      await page.locator('[data-estimate="custom:account"]').click();assert.equal(await page.locator('#estimate-window').inputValue(),'five');await page.locator('#estimate-confirm').check();await page.evaluate(()=>feature.delaySample=true);await page.locator('#estimate-start').click();
      await page.waitForFunction(()=>feature.releaseSample);assert.match(await page.locator('#quota-progress').textContent(),/同步来源 · 本机.*秒/);assert(await page.locator('#estimate-start').isDisabled());
      await page.evaluate(()=>feature.releaseSample());await page.locator('#estimate-stop').waitFor();
      for(const value of ['$20.00 USD','$100.00 USD','$110.00 USD'])assert.equal(await page.locator('#quota-dialog').getByText(value,{exact:true}).count(),1);
      await page.screenshot({path:path.join(output,surface+'-five-hour.png')});await page.locator('#estimate-stop').click();await page.locator('#estimate-start').waitFor();await page.locator('#quota-close').click();
      assert.deepEqual(errors,[]);await page.screenshot({path:path.join(output,surface+'-panel.png')});await page.close();
    }
    console.log('New features: password cleanup, proxy tests, mapping focus, top save/reprice, saves independent of dashboard latency, footer update/latency, cross-window sampling guard and three 5h/7d values passed');
  }finally{await browser?.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
