const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
function fixture(){
  window.setInterval=()=>0;
  const settings={version:7,agents:[{provider:'codex',enabled:true,machineIds:['local','local','server','offline','paused']}],accounts:[{id:'a',provider:'codex',name:'个人',quotaEnabled:true},{id:'b',provider:'codex',name:'工作',quotaEnabled:true},{id:'old',provider:'codex',name:'归档',archived:true,quotaEnabled:true}],sources:[{id:'local',name:'本机目录',provider:'codex',path:'/local',codexHomeId:'h1',accountId:'',enabled:true},{id:'remote',name:'服务器目录',provider:'codex',path:'/remote',codexHomeId:'h2',accountId:'',hostId:'server',enabled:true},{id:'offline',name:'离线目录',provider:'codex',path:'/offline',codexHomeId:'h3',accountId:'',hostId:'offline',enabled:true}],hosts:[{id:'server',name:'144 服务器',enabled:true},{id:'offline',name:'离线服务器',enabled:true},{id:'paused',name:'已停用',enabled:false}],modelMappings:{},proxy:{mode:'direct'},refreshSeconds:300,serverRefreshSeconds:10};
  const summary={tokens:{input:0,output:0,cacheRead:0,cacheWrite:0},total:0,cost:0,pricedTokens:0,events:0};
  window.targetsTest={calls:[],linked:false,holdOffline:true,settings};
  const t=targetsTest;
  t.usingWork=false;t.holdStatus=false;
  function statusRows(){return settings.accounts.flatMap(a=>['local','local-copy','remote','offline'].map(sourceId=>({accountKey:'codex:'+a.id,sourceId,machineId:sourceId.startsWith('local')?'local':sourceId==='remote'?'server':'offline',current:sourceId==='offline'?null:sourceId==='remote'&&(t.usingWork?a.id==='b':a.id==='a'),credential:sourceId==='offline'?null:sourceId==='remote',error:sourceId==='offline'?'离线':null})));}
  function rows(){return settings.accounts.filter(a=>!a.archived).flatMap(a=>settings.sources.map(s=>({accountKey:'codex:'+a.id,accountName:a.name,provider:'codex',sourceId:s.id,sourceName:s.name,path:s.path,machineId:s.hostId||'local',machineName:s.hostId?settings.hosts.find(h=>h.id===s.hostId).name:'本机',profileId:s.id==='offline'||s.id==='remote'&&!t.linked?null:s.codexHomeId+':'+a.id,current:s.id==='remote'&&a.id==='a',credential:s.id==='offline'?null:s.id==='remote'?true:false,canWake:s.id!=='offline'&&(s.id!=='remote'||t.linked),canSwitch:s.id==='remote'&&a.id==='b'&&t.linked})));}
  window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){
    if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'floating',effectiveMode:'floating',sessions:[],summary:'Idle',panelOpen:true};
    if(command!=='engine_call')return {};
    const {method,params}=args;t.calls.push({method,params});
    if(method==='hello')return {version:'fixture'};
    if(method==='settings.get')return structuredClone(settings);
    if(method==='accounts.status.get')return statusRows();
    if(method==='accounts.status.refresh'){const row=statusRows().find(r=>r.accountKey===params.accountKey&&r.sourceId===params.sourceId);if(t.holdStatus)await new Promise(resolve=>t.releaseStatus=resolve);return row;}
    if(method==='accounts.targets')return rows().filter(r=>!params.accountKey||r.accountKey===params.accountKey);
    if(method==='codexAuth.reconcile'){
      if(params.sourceId==='offline'){
        if(t.holdOffline)await new Promise(resolve=>t.releaseOffline=()=>{t.holdOffline=false;resolve();});
        throw new Error('离线，可重试');
      }
      if(params.sourceId==='remote')t.linked=true;
      return {settings,issues:[]};
    }
    if(method==='wakeups.probe')return new Promise(resolve=>t.releaseProbe=()=>resolve({models:[{id:'STALE',efforts:['high']}],recommendedModel:'STALE',efforts:['high'],timezone:'UTC',message:'OLD'}));
    if(method==='wakeups.save')return {task:params.task};
    if(method==='dashboard')return {summary,quotas:settings.accounts.filter(a=>!a.archived).map(a=>({provider:'codex',accountId:a.id,name:a.name,windows:[],updatedAt:Date.now()/1000,origin:'live'})),sources:[],models:[],dayModels:[],modelOptions:[],trendDays:[],heatmap:[],pricingGaps:[]};
    return [];
  }}};
}
(async()=>{
  const root=path.resolve(__dirname,'../apps/desktop/web');
  const server=http.createServer((req,res)=>{if(require('./ui-test-helpers.cjs').serveFont(req,res))return;const file=path.join(root,path.basename(new URL(req.url,'http://localhost').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const browser=await chromium.launch({headless:true});
  try{
    const page=await browser.newPage({reducedMotion:'reduce'}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
    assert.equal(await page.locator('[data-switch-account="codex:old"]').count(),0);
    await page.locator('[data-device-counts="codex:a"]').getByText('已登录设备1/3',{exact:true}).waitFor();
    await page.locator('[data-device-counts="codex:a"]').getByText('使用中设备1/3',{exact:true}).waitFor();
    assert.match(await page.locator('[data-device-counts="codex:b"]').innerText(),/使用中设备0\/3/);
    assert.match(await page.locator('[data-device-counts="codex:b"]').innerText(),/有设备待确认/);
    // Font swaps and viewport reflow can otherwise be measured on different frames.
    await page.evaluate(()=>document.fonts.ready);
    await page.setViewportSize({width:450,height:720});
    await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
    fs.mkdirSync(path.resolve(__dirname,'../.local/ui-previews'),{recursive:true});await page.screenshot({path:path.resolve(__dirname,'../.local/ui-previews/device-counts.png')});
    const switchBox=await page.locator('[data-switch-account="codex:b"]').boundingBox(),countsBox=await page.locator('[data-device-counts="codex:b"]').boundingBox();
    assert(countsBox.x>switchBox.x+switchBox.width,'counts appear to the right of the switch button');
    assert(countsBox.x+countsBox.width<=450,'counts fit the narrow panel');
    await page.waitForFunction(()=>targetsTest.calls.filter(c=>c.method==='accounts.status.refresh').length>=6);
    await page.waitForFunction(()=>!AieyesAgentSettings.statusBusy);
    // A source refresh supersedes an older global observation without changing the page.
    await page.evaluate(()=>{targetsTest.holdStatus=true;void AieyesAgentSettings.refreshStatuses(true);});
    await page.waitForFunction(()=>targetsTest.releaseStatus);
    await page.evaluate(async()=>{targetsTest.holdStatus=false;targetsTest.usingWork=true;await AieyesAgentSettings.refreshCodexSource('remote');targetsTest.releaseStatus();});
    await page.locator('[data-device-counts="codex:b"]').getByText('使用中设备1/3',{exact:true}).waitFor();
    await page.waitForTimeout(50);
    assert.match(await page.locator('[data-device-counts="codex:a"]').innerText(),/使用中设备0\/3/,'late status cannot restore the previous account');

    await page.evaluate(()=>{AieyesCodexAuth.open=(...args)=>targetsTest.opened=args;AieyesCodexAuth.html=()=>'<p>登录或切换确认</p>';AieyesCodexAuth.bind=()=>{};});
    await page.locator('[data-switch-account="codex:b"]').click();
    const remote=page.locator('#account-targets .list-row').filter({hasText:'144 服务器'});
    await remote.getByRole('button',{name:'切换',exact:true}).waitFor();
    assert.equal(await remote.getByRole('button').isEnabled(),true,'offline machine does not block a ready machine');
    await remote.getByRole('button').click();assert.deepEqual(await page.evaluate(()=>targetsTest.opened),['b','remote','switch']);
    await page.locator('#target-back').click();
    await page.evaluate(()=>targetsTest.releaseOffline());
    await page.locator('#target-refresh:not([disabled])').waitFor();
    await page.locator('#account-targets .list-row').filter({hasText:'本机'}).getByRole('button',{name:'登录',exact:true}).click();
    assert.deepEqual(await page.evaluate(()=>targetsTest.opened),['b','local','login']);
    await page.locator('#target-close').click();
    await page.evaluate(()=>{state.page='settings';state.settingsTab='accounts';renderSettings();});
    await page.getByText('已登录设备1/3 · 使用中设备1/3 · 有设备待确认',{exact:true}).waitFor();
    await page.evaluate(()=>{targetsTest.settings.hosts.find(h=>h.id==='offline').enabled=false;AieyesAgentSettings.accept(structuredClone(targetsTest.settings));state.page='agent';render();});
    await page.locator('[data-device-counts="codex:b"]').getByText('使用中设备1/2',{exact:true}).waitFor();
    await page.evaluate(()=>{targetsTest.settings.hosts.find(h=>h.id==='offline').enabled=true;AieyesAgentSettings.accept(structuredClone(targetsTest.settings));render();});
    await page.evaluate(()=>{state.page='settings';state.settingsTab='wakeups';renderSettings();});await page.locator('#wake-add:not([disabled])').click();
    await page.locator('#wake-account').selectOption('codex:a');await page.locator('#wake-source').selectOption('remote');
    assert.equal(await page.locator('[name=codexProfileId]').inputValue(),'h2:a','server login is selectable after reconciliation');
    await page.locator('#wake-probe').click();await page.waitForFunction(()=>targetsTest.releaseProbe);
    await page.locator('#wake-account').selectOption('codex:b');await page.evaluate(()=>targetsTest.releaseProbe());
    await page.waitForFunction(()=>!document.querySelector('#wake-probe').disabled);
    assert.equal(await page.locator('#wake-model').inputValue(),'','old model response cannot cross accounts');
    assert.equal(await page.locator('#wake-models option').count(),0);
    await page.locator('#wake-source').selectOption('remote');await page.locator('#wake-model').fill('chosen');await page.locator('#editor-form button[type=submit]').click();
    await page.waitForFunction(()=>!document.querySelector('#editor').open);
    const saved=await page.evaluate(()=>targetsTest.calls.find(c=>c.method==='wakeups.save').params.task);
    assert.equal(saved.sourceId,'remote');assert.equal(saved.codexProfileId,'h2:b');
    assert.deepEqual(errors,[]);
    console.log('Account targets: deduplicated device counts, settings parity, offline/paused devices, late status rejection, independent machines, archived eligibility, login/switch routing, reconciled wakeup selection and stale capabilities passed');
  }finally{await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
