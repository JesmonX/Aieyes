/* Fresh-install race: another window's operation invalidates shared data while this window is still booting.
   The shared refresh then fetches settings before boot assigns them, and must not report an error. */
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),os=require('node:os'),http=require('node:http'),readline=require('node:readline');
const {spawn,execFileSync}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const {serveFont}=require('./ui-test-helpers.cjs');
const project=path.resolve(__dirname,'..'),root=fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-boot-')),home=path.join(root,'home');fs.mkdirSync(home);
// Empty data directory and home: the real core starts as a fresh install, without the developer's agent data.
const core=spawn(path.join(project,'target/debug/aieyes-core'),['--data-dir',path.join(root,'data')],{stdio:['pipe','pipe','pipe'],env:{...process.env,AIEYES_AUTO_SCAN:'off',HOME:home,USERPROFILE:home,CLAUDE_CONFIG_DIR:path.join(home,'.claude'),CODEX_HOME:path.join(home,'.codex')}});
// `kill()` requests termination; Windows can retain SQLite/credential handles
// until the process and its stdio have actually closed.
const coreClosed=new Promise(resolve=>core.once('close',resolve));
let counter=0,stderr='',settingsReads=0;const pending=new Map();core.stderr.on('data',b=>stderr+=b);
readline.createInterface({input:core.stdout}).on('line',line=>{const r=JSON.parse(line),p=pending.get(r.id);if(!p)return;pending.delete(r.id);if(r.error)p.reject(new Error(r.error.message));else p.resolve(r.result);});
core.on('exit',()=>{for(const p of pending.values())p.reject(new Error('Core exited: '+stderr));pending.clear();});
const rpc=(method,params={})=>new Promise((resolve,reject)=>{const id=++counter;pending.set(id,{resolve,reject});core.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');});
async function invoke(method,params){
  // Hold boot's settings read so that the invalidation arrives first.
  if(method==='settings.get'&&++settingsReads===1)await new Promise(resolve=>setTimeout(resolve,1500));
  if(['quotas.refresh','sources.scan','hosts.sample'].includes(method))return [];
  if(method==='sessions.list')return {sessions:[],warnings:[]};
  if(method.startsWith('network.'))return {status:'ok',sites:[],testedAt:Date.now()/1000};
  return rpc(method,params);
}
function browserFixture(panel){
  window.setInterval=()=>0;window.testListeners={};
  window.__TAURI__={event:{async listen(name,handler){(window.testListeners[name]??=[]).push(handler);return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){
    if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'floating',effectiveMode:'floating',summary:'Idle',sessions:[],panelOpen:panel,nativePet:true};
    if(command==='desktop_panel')return {panelOpen:args.open,nativePet:true};
    if(command==='engine_call')return window.testRPC(args.method,args.params??{});return {};
  }}};
}
// A later login must reach the visible settings DOM, including when the first
// status refresh starts with zero account keys. Other settings can still have drafts.
async function lateDiscovery(browser,url,surface,panel){
  const directory=path.join(root,'late-'+surface),fixtureHome=path.join(directory,'home'),data=path.join(directory,'data'),binary=path.join(project,'target/debug/aieyes-core');
  fs.mkdirSync(fixtureHome,{recursive:true});
  const env={...process.env,AIEYES_AUTO_SCAN:'off',HOME:fixtureHome,USERPROFILE:fixtureHome,CLAUDE_CONFIG_DIR:path.join(fixtureHome,'.claude'),CODEX_HOME:path.join(fixtureHome,'.codex')};
  const call=(method,params={})=>JSON.parse(execFileSync(binary,['--data-dir',data,'--call',method,JSON.stringify(params)],{env,encoding:'utf8'}));
  const settings=call('settings.get');settings.hosts=[];
  for(const agent of settings.agents){agent.enabled=agent.provider==='claude';agent.machineIds=agent.enabled?['local']:[];}
  call('settings.save',settings);
  // Only this disposable process opts in. Claude metadata and tokens below are synthetic.
  const child=spawn(binary,['--data-dir',data],{env:{...env,AIEYES_AUTO_SCAN:'on'},stdio:['pipe','pipe','pipe']}),closed=new Promise(resolve=>child.once('close',resolve));
  let id=0,stderr='',page;const pending=new Map(),calls=[],errors=[],forwarding=[];
  child.stderr.on('data',chunk=>stderr+=chunk);
  readline.createInterface({input:child.stdout}).on('line',line=>{
    const response=JSON.parse(line);
    if(response.method==='runtime.changed'){
      if(page&&!page.isClosed())forwarding.push(page.evaluate(payload=>{for(const listener of window.testListeners?.['runtime:changed']??[])listener({payload});},response.params).catch(error=>errors.push(String(error))));
      return;
    }
    const waiter=pending.get(response.id);if(!waiter)return;pending.delete(response.id);response.error?waiter.reject(new Error(response.error.message)):waiter.resolve(response.result);
  });
  child.on('exit',()=>{for(const waiter of pending.values())waiter.reject(new Error(stderr||'Core exited'));pending.clear();});
  const rpc=(method,params={})=>new Promise((resolve,reject)=>{const request=++id;pending.set(request,{resolve,reject});child.stdin.write(JSON.stringify({jsonrpc:'2.0',id:request,method,params})+'\n');});
  const writeLogin=name=>{
    fs.mkdirSync(path.join(fixtureHome,'.claude'),{recursive:true});
    fs.writeFileSync(path.join(fixtureHome,'.claude.json'),JSON.stringify({oauthAccount:{accountUuid:name+'-uuid',organizationUuid:name+'-org',emailAddress:name+'@example.test'}}));
    fs.writeFileSync(path.join(fixtureHome,'.claude/.credentials.json'),JSON.stringify({claudeAiOauth:{accessToken:'fixture-only-'+name,expiresAt:4102444800000}}));
  };
  const settle=()=>page.waitForFunction(()=>!externalRefresh&&!state.dashboardPending&&!window.AieyesAgentSettings.statusBusy);
  try{
    await rpc('accounts.autoScan',{targets:[['claude','local']]});
    assert.equal((await rpc('settings.get')).accounts.length,0);
    page=await browser.newPage({viewport:panel?{width:450,height:800}:{width:1180,height:900}});page.on('pageerror',error=>errors.push(String(error)));
    await page.exposeFunction('testRPC',async(method,params)=>{
      calls.push({method,params});
      if(['quotas.refresh','sources.scan','hosts.sample','wakeups.list'].includes(method))return [];
      if(method==='claudeAuth.keychain')return {supported:true,asked:false,available:false};
      if(method==='sessions.list')return {sessions:[],warnings:[]};
      if(method.startsWith('network.'))return {};
      return rpc(method,params);
    });
    await page.addInitScript(browserFixture,panel);await page.goto(url+'/'+surface);
    await page.waitForFunction(()=>typeof state!=='undefined'&&state.settings&&state.dashboard&&!state.busy&&!window.AieyesAgentSettings.statusBusy);
    assert.equal(await page.evaluate(()=>state.settings.accounts.length),0);
    if(!panel)await page.evaluate(()=>{state.page='settings';state.settingsTab='integrations';state.settingsAgent='claude';render();});
    writeLogin('late');
    await page.evaluate(()=>window.AieyesAgentSettings.refreshStatuses(true,null,true));
    await page.waitForFunction(()=>state.settings.accounts.some(account=>account.name==='late@example.test'));
    await settle();
    await page.waitForFunction(()=>document.body.textContent.includes('late@example.test'),null,{timeout:10000});
    assert(calls.some(call=>call.method==='accounts.status.refreshBatch'&&call.params.accountKeys.length===0));
    assert.equal(await page.locator('[data-native-scan-agent],[data-native-scan-machine]').count(),0);
    if(!panel){
      assert.equal(await page.locator('[data-account-edit]').count(),1,'A settings revision inserts the first account without navigation');
      await page.evaluate(()=>{state.settingsTab='general';state.generalTab='refresh';renderSettings();});
      const savedSeconds=(await rpc('settings.get')).refreshSeconds,draft=String(savedSeconds+17);
      await page.locator('#field-refreshSeconds').fill(draft);
      const savesBefore=calls.filter(call=>call.method==='settings.patch').length;
      writeLogin('changed');
      await page.evaluate(()=>window.AieyesAgentSettings.refreshStatuses(true,null,true));
      await page.waitForFunction(()=>state.settings.accounts.some(account=>account.name==='changed@example.test'));
      await settle();
      assert.equal(await page.locator('#field-refreshSeconds').inputValue(),draft,'Shared settings refresh preserves the focused unsaved field');
      assert.equal(await page.locator('#general-form').getAttribute('data-dirty'),'true');
      assert.equal((await rpc('settings.get')).refreshSeconds,savedSeconds,'The draft has not been submitted');
      assert.equal(calls.filter(call=>call.method==='settings.patch').length,savesBefore);
    }
    assert.deepEqual(errors,[]);
  }finally{
    await Promise.allSettled(forwarding);await page?.close();child.kill();await closed;
  }
}
(async()=>{
  const web=path.join(project,'apps/desktop/web');
  const server=http.createServer((req,res)=>{if(serveFont(req,res))return;const file=path.join(web,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'text/plain');fs.createReadStream(file).pipe(res);});
  await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
  try {
    browser=await chromium.launch({headless:true});
    for(const [surface,panel] of [['index.html',false],['floating.html',true]]){
      settingsReads=0;
      const page=await browser.newPage({viewport:panel?{width:450,height:720}:{width:1180,height:900}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));
      await page.exposeFunction('testRPC',invoke);await page.addInitScript(browserFixture,panel);
      await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);
      await page.waitForFunction(()=>typeof state!=='undefined'&&state.revisionEvents);
      assert.equal(await page.evaluate(()=>state.settings),null,`${surface}: the invalidation arrives before boot assigns settings`);
      await page.evaluate(()=>{for(const handler of window.testListeners['desktop:invalidate']??[])handler({payload:{method:'settings.patch',operationId:'other-window'}});});
      await page.waitForFunction(()=>state.settings&&state.dashboard);await page.waitForTimeout(500);
      assert.ok(settingsReads>=2,`${surface}: the shared refresh read settings during boot`);
      assert.deepEqual(errors,[],`${surface}: no page errors`);
      const message=await page.locator('#message').textContent();
      assert.doesNotMatch(message,/TypeError|操作未完成/,`${surface}: the early shared refresh reports no error`);
      await page.close();
    }
    for(const [surface,panel] of [['index.html',false],['floating.html',true]])await lateDiscovery(browser,`http://127.0.0.1:${server.address().port}`,surface,panel);
    console.log('Boot refresh races, late native discovery on both surfaces and unsaved settings preservation passed');
  } finally {
    await browser?.close();
    await new Promise(resolve=>server.close(resolve));
    core.kill();
    await coreClosed;
    fs.rmSync(root,{recursive:true,force:true,maxRetries:10,retryDelay:100});
  }
})().catch(error=>{console.error(error);process.exitCode=1;});
