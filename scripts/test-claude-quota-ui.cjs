/* Claude official quota snapshot -> real core cache/dashboard -> both browser surfaces.
   quota.rs tests validate the raw /api/oauth/usage conversion (scripts/fixtures/claude-oauth-usage.json).
   The Keychain answer row and its retry are stubbed here: a non-startup claudeAuth.keychain would read the
   developer's real Keychain, so it never reaches core (claude_keychain.rs covers that logic). */
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),os=require('node:os'),http=require('node:http'),readline=require('node:readline');
const {DatabaseSync}=require('node:sqlite'),{spawn}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright'),{serveFont,revealControl}=require('./ui-test-helpers.cjs');
const project=path.resolve(__dirname,'..'),root=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-claude-quota-')));
const core=spawn(path.join(project,'target/debug/aieyes-core'),['--data-dir',path.join(root,'data')],{stdio:['pipe','pipe','pipe'],env:{...process.env,AIEYES_AUTO_SCAN:'off',HOME:root,USERPROFILE:root,CODEX_HOME:path.join(root,'.codex')}});
const pending=new Map(),retries=[];let sequence=0,stderr='',db,keychain=null,quotaRefreshes=0;
core.stderr.on('data',b=>stderr+=b);
readline.createInterface({input:core.stdout}).on('line',line=>{const r=JSON.parse(line),p=pending.get(r.id);if(!p)return;pending.delete(r.id);r.error?p.reject(new Error(r.error.message)):p.resolve(r.result);});
core.on('exit',()=>{for(const p of pending.values())p.reject(new Error(stderr||'Core exited'));pending.clear();});
function rpc(method,params={}){return new Promise((resolve,reject)=>{const id=++sequence;pending.set(id,{resolve,reject});core.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');});}
async function invoke(method,params={}){
  if(method==='quotas.refresh')quotaRefreshes++;
  if(method==='claudeAuth.keychain'&&params.startup!==true){retries.push(params);keychain={askedAt:Math.floor(Date.now()/1000),granted:true,result:'granted'};return {supported:true,asked:true,askedAt:keychain.askedAt,granted:true,result:'granted',available:true};}
  if(method==='settings.get'&&keychain)return {...await rpc(method,params),claudeKeychain:keychain};
  if(['quotas.refresh','hosts.sample','wakeups.list','accounts.status.refresh','accounts.devices'].includes(method))return [];
  if(method==='sessions.list')return {sessions:[],sources:[],warnings:[]};
  if(method.startsWith('network.'))return {status:'ok',sites:[],testedAt:Date.now()/1000};
  return rpc(method,params);
}
function bridge(){window.setInterval=()=>0;window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){if(command==='engine_call')return window.testRPC(args.method,args.params??{});if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'floating',effectiveMode:'floating',sessions:[],panelOpen:true};return {};}}};}
const wallet0={currency:'USD',total:'50.00',balance:'37.66',monthlyLimitEnabled:true,monthlyLimit:'50.00',monthlyUsed:'12.34'};
const week=10080,windows=[{id:'five_hour',groupId:'five_hour',groupName:'5h',name:'5h',usedPercent:12.5,windowMinutes:300},{id:'seven_day',groupId:'seven_day',groupName:'7d',name:'7d',usedPercent:40,windowMinutes:week},{id:'seven_day_opus',groupId:'seven_day_opus',groupName:'Opus · 7d',name:'Opus · 7d',usedPercent:55,windowMinutes:week},{id:'limits:sonnet',groupId:'limits:sonnet',groupName:'Sonnet · 7d',name:'Sonnet · 7d',usedPercent:30,windowMinutes:week}];
let version=0;
function seed(wallet=wallet0,extra={}){
  const q={provider:'claude',accountId:'claude',sourceId:'source',name:'Claude',updatedAt:Math.floor(Date.now()/1000)+version++,origin:'live',windows,balances:[],extraUsage:wallet,...extra};
  db.prepare('INSERT INTO quotas(account_key,payload) VALUES(?,?) ON CONFLICT(account_key) DO UPDATE SET payload=excluded.payload').run('claude:claude',JSON.stringify(q));return q;
}
(async()=>{
  let browser,server;
  try{
    // No ~/.claude under the fixture HOME: the startup request must not touch the Keychain.
    const before=await rpc('claudeAuth.status');assert.equal(before.asked,false);assert.equal(before.available,false);
    const startup=await rpc('claudeAuth.keychain',{startup:true});assert.equal(startup.asked,false);assert.equal((await rpc('settings.get')).claudeKeychain,undefined);
    const s=await rpc('settings.get');s.accounts=[{id:'claude',provider:'claude',name:'Claude',quotaEnabled:true}];s.sources=[{id:'source',provider:'claude',name:'Claude',accountId:'claude',enabled:true,path:path.join(root,'.claude')}];s.hosts=[];
    for(const agent of s.agents){agent.enabled=agent.provider==='claude';agent.machineIds=['local'];}await rpc('settings.save',s);
    db=new DatabaseSync(path.join(root,'data/aieyes.sqlite'));seed();
    assert.deepEqual((await rpc('dashboard')).quotas[0].extraUsage,wallet0);
    const web=path.join(project,'apps/desktop/web'),output=path.join(project,'.local/claude-quota');fs.mkdirSync(output,{recursive:true});
    server=http.createServer((req,res)=>{if(serveFont(req,res))return;const file=path.join(web,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});await new Promise(r=>server.listen(0,'127.0.0.1',r));browser=await chromium.launch({headless:true});
    for(const surface of ['index.html','floating.html']){
      seed();const page=await browser.newPage({viewport:{width:surface==='floating.html'?450:1120,height:900}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.exposeFunction('testRPC',invoke);await page.addInitScript(bridge);await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
      const card=page.locator('[data-quota="claude:claude"]:visible');await card.waitFor();const wallet=card.locator('[data-extra-usage]');await wallet.getByText('USD 37.66',{exact:true}).waitFor();
      if(!await card.locator('.quota-disclosure').evaluate(el=>el.open))await card.locator('.quota-title').click();
      assert.match(await wallet.textContent(),/钱包总额USD 50.00/);assert.match(await wallet.textContent(),/本月扣费USD 12.34/);assert.match(await wallet.textContent(),/月度上限 USD 50.00/);
      const text=await card.textContent();assert.match(text,/Sonnet · 7d/);assert.match(text,/Opus · 7d/);assert.match(text,/剩余 70(?:\.0)?%/);
      // Per-model limits keep their own group, so the card keeps the ungrouped layout and estimate entry.
      assert.equal(await page.evaluate(()=>quotaGroupsWindows(state.dashboard.quotas[0])),false);
      await page.screenshot({animations:'disabled',path:path.join(output,surface+'-complete.png')});
      seed({...wallet0,balance:null,monthlyLimitEnabled:false,monthlyLimit:null,total:'2.50',monthlyUsed:'2.50'});await page.evaluate(()=>loadDashboard());await wallet.getByText('月度上限未启用',{exact:true}).waitFor();
      seed(null,{windows:[],origin:'local',error:null,notice:'未授权读取钥匙串，仍可查看本地用量'});await page.evaluate(()=>loadDashboard());await page.waitForFunction(()=>!document.querySelector('[data-extra-usage]'));assert.match(await card.textContent(),/未授权读取钥匙串，仍可查看本地用量/);assert.doesNotMatch(await card.textContent(),/额度更新失败|显示本地估算/);assert.equal(await page.evaluate(()=>quotaNeedsAttention(state.dashboard.quotas[0])),false);
      await page.setViewportSize({width:360,height:900});seed();await page.evaluate(()=>loadDashboard());await wallet.getByText('USD 37.66',{exact:true}).waitFor();assert(await wallet.evaluate(el=>el.scrollWidth<=el.clientWidth+1));
      assert.deepEqual(errors,[]);await page.close();
    }
    {
      // Settings: the recorded answer, its retry entry, then the official quota is refreshed.
      keychain={askedAt:1,granted:false,result:'denied'};
      const page=await browser.newPage({viewport:{width:1120,height:900}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.exposeFunction('testRPC',invoke);await page.addInitScript(bridge);await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
      await page.evaluate(()=>{state.page='settings';state.settingsTab='sources';state.settingsAgent='claude';render();});
      await revealControl(page,'[data-claude-keychain]');const row=page.locator('[data-claude-keychain-status]');
      assert.equal(await row.getAttribute('data-claude-keychain-status'),'denied');assert.match(await row.innerText(),/已拒绝钥匙串授权，仍可查看本地用量/);
      keychain={askedAt:1,granted:false,result:'missing'};await page.evaluate(async()=>{window.AieyesAgentSettings.accept(await api('settings.get'));renderSettings(false);});
      assert.equal(await row.getAttribute('data-claude-keychain-status'),'missing');assert.match(await row.innerText(),/未登录或使用 API key，仍可查看本地用量/);
      await page.screenshot({animations:'disabled',path:path.join(output,'settings-missing.png')});
      const before=quotaRefreshes;await revealControl(page,'[data-claude-keychain]');await page.locator('[data-claude-keychain]').click();
      await page.waitForFunction(()=>document.querySelector('[data-claude-keychain-status]')?.dataset.claudeKeychainStatus==='granted');
      assert.match(await row.innerText(),/钥匙串已获授权，显示官方额度/);assert.equal(await page.locator('[data-claude-keychain]').count(),0);
      assert.deepEqual(retries,[{}]);for(let i=0;i<100&&quotaRefreshes===before;i++)await new Promise(r=>setTimeout(r,20));assert(quotaRefreshes>before,'A new grant refreshes the official quota');
      assert.deepEqual(errors,[]);await page.close();
    }
    console.log('Claude quota: startup keychain guard, real cache/dashboard, both surfaces, per-model limits, extra usage wallet, optional keychain notice, narrow layout and Keychain answer row with retry passed');
  } finally {
    if(browser)await browser.close();if(server)await new Promise(r=>server.close(r));if(db)db.close();core.kill();await new Promise(r=>core.exitCode!==null?r():core.once('exit',r));fs.rmSync(root,{recursive:true,force:true});
  }
})().catch(e=>{console.error(e);process.exitCode=1;});
