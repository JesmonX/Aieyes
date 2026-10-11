const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
function fixture(){
  window.setInterval=()=>0;
  const settings={version:7,agents:[{provider:'codex',enabled:true,machineIds:['local','server','offline']}],accounts:[{id:'a',provider:'codex',name:'个人',quotaEnabled:true},{id:'b',provider:'codex',name:'工作',quotaEnabled:true},{id:'old',provider:'codex',name:'归档',archived:true,quotaEnabled:true}],sources:[{id:'local',name:'本机目录',provider:'codex',path:'/local',codexHomeId:'h1',accountId:'',enabled:true},{id:'remote',name:'服务器目录',provider:'codex',path:'/remote',codexHomeId:'h2',accountId:'',hostId:'server',enabled:true},{id:'offline',name:'离线目录',provider:'codex',path:'/offline',codexHomeId:'h3',accountId:'',hostId:'offline',enabled:true}],hosts:[{id:'server',name:'144 服务器',enabled:true},{id:'offline',name:'离线服务器',enabled:true}],modelMappings:{},proxy:{mode:'direct'},refreshSeconds:300,serverRefreshSeconds:10};
  const summary={tokens:{input:0,output:0,cacheRead:0,cacheWrite:0},total:0,cost:0,pricedTokens:0,events:0};
  window.targetsTest={calls:[],linked:false,holdOffline:true,settings};
  const t=targetsTest;
  function rows(){return settings.accounts.filter(a=>!a.archived).flatMap(a=>settings.sources.map(s=>({accountKey:'codex:'+a.id,accountName:a.name,provider:'codex',sourceId:s.id,sourceName:s.name,path:s.path,machineId:s.hostId||'local',machineName:s.hostId?settings.hosts.find(h=>h.id===s.hostId).name:'本机',profileId:s.id==='offline'||s.id==='remote'&&!t.linked?null:s.codexHomeId+':'+a.id,current:s.id==='remote'&&a.id==='a',credential:s.id==='offline'?null:s.id==='remote'?true:false,canWake:s.id!=='offline'&&(s.id!=='remote'||t.linked),canSwitch:s.id==='remote'&&a.id==='b'&&t.linked})));}
  window.__TAURI__={event:{async listen(){return()=>{};}},core:{async invoke(command,args){
    if(command==='desktop_info')return {platform:'windows',mode:'floating',effectiveMode:'floating',sessions:[],summary:'Idle',panelOpen:true};
    if(command!=='engine_call')return {};
    const {method,params}=args;t.calls.push({method,params});
    if(method==='hello')return {version:'fixture'};
    if(method==='settings.get')return structuredClone(settings);
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
    const page=await browser.newPage(),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
    assert.equal(await page.locator('[data-switch-account="codex:old"]').count(),0);
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
    console.log('Account targets: independent machines, archived eligibility, login/switch routing, reconciled wakeup selection and stale capabilities passed');
  }finally{await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
