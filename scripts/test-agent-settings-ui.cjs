/* Configuration calls use the real core and a disposable database; auth is simulated. */
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),os=require('node:os'),http=require('node:http'),readline=require('node:readline');
const {spawn,execFileSync}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const project=path.resolve(__dirname,'..'),root=fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-settings-'));
const fixture={version:2,sources:[{id:'codex-local',name:'Codex · 本机',provider:'codex',path:root+'/codex',accountId:'',enabled:true,codexHomeId:'home'}],accounts:[{id:'a',name:'个人 Codex',provider:'codex',quotaEnabled:true,quotaProfileId:'home:one'},{id:'b',name:'工作 Codex',provider:'codex',quotaEnabled:true,quotaProfileId:'home:two'},{id:'old',name:'归档账户',provider:'codex',quotaEnabled:true,archived:true}],hosts:[{id:'server',name:'测试服务器',target:'fixture.invalid',enabled:true}],modelMappings:{}};
const historyEvent={id:'historic-event',provider:'codex',accountId:'history',sourceId:'removed-source',sessionId:'historic-session',model:'fixture-model',timestamp:Math.floor(Date.now()/1000),tokens:{input:100},attribution:'import'};
execFileSync(process.platform==='win32'?'python':'python3',['-c',`import sqlite3,json,sys
c=sqlite3.connect(sys.argv[1])
c.execute("CREATE TABLE kv(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
c.execute("INSERT INTO kv VALUES(?,?)",("settings",sys.argv[2]))
c.execute("INSERT INTO kv VALUES(?,?)",("deleted-account:codex:history",json.dumps(dict(id='history',provider='codex',name='旧账户历史'))))
c.execute("CREATE TABLE events(id TEXT PRIMARY KEY,provider TEXT NOT NULL,account_id TEXT NOT NULL,model TEXT NOT NULL,stamp INTEGER NOT NULL,payload TEXT NOT NULL,cost REAL NOT NULL,priced_tokens INTEGER NOT NULL,price TEXT)")
e=json.loads(sys.argv[3])
c.execute("INSERT INTO events VALUES(?,?,?,?,?,?,0,0,NULL)",(e['id'],e['provider'],e['accountId'],e['model'],e['timestamp'],sys.argv[3]))
c.commit()`,path.join(root,'aieyes.sqlite'),JSON.stringify(fixture),JSON.stringify(historyEvent)]);
const claudeRoot=path.join(root,'claude-profile');fs.mkdirSync(claudeRoot);
const core=spawn(path.join(project,'target/debug/aieyes-core'),['--data-dir',root],{stdio:['pipe','pipe','pipe'],env:{...process.env,CLAUDE_CONFIG_DIR:claudeRoot}});
let counter=0;const pending=new Map();let stderr='';core.stderr.on('data',b=>stderr+=b);
readline.createInterface({input:core.stdout}).on('line',line=>{const r=JSON.parse(line),p=pending.get(r.id);if(!p)return;pending.delete(r.id);if(r.error)p.reject(new Error(r.error.message));else p.resolve(r.result);});
core.on('exit',()=>{for(const p of pending.values())p.reject(new Error('Core exited: '+stderr));pending.clear();});
function rpc(method,params={}){return new Promise((resolve,reject)=>{const id=++counter;pending.set(id,{resolve,reject});core.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');});}
const profile=id=>({id,name:id==='one'?'个人 Codex':'工作 Codex',current:id==='one',identity:{key:id,workspace:'workspace',email:id+'@example.test',plan:'plus'}});
let authDone=false,authCancelled=false,unknown=false,failSave=false,loggedIn=true,authFailed=false;const calls=[],secrets=new Map();let secretId=0,saveDelay=0;
async function invoke(method,params){
  calls.push({method,params});
  // Keep the operating-system credential store outside this disposable UI test.
  if(method==='hosts.credentials.save'){const passwordRef='ssh-fixture-'+(++secretId);secrets.set(passwordRef,params.password);return {passwordRef};}
  if(method==='hosts.credentials.delete'){assert(secrets.delete(params.passwordRef));return {};}
  if(['settings.patch','sources.configure'].includes(method)&&failSave)throw new Error('模拟保存失败，输入已保留');
  if(method==='settings.patch'&&saveDelay)await new Promise(resolve=>setTimeout(resolve,saveDelay));
  if(method==='accounts.status.refreshBatch'||method==='accounts.status.get'){
    const settings=await rpc('settings.get'),rows=[];
    for(const a of settings.accounts.filter(a=>!a.archived&&(!params.accountKeys||params.accountKeys.includes(a.provider+':'+a.id))))for(const source of settings.sources.filter(s=>s.provider===a.provider&&s.enabled&&(!params.sourceIds||params.sourceIds.includes(s.id))))rows.push(await invoke('accounts.status.refresh',{accountKey:a.provider+':'+a.id,sourceId:source.id}));
    return rows;
  }
  if(method==='accounts.status.refresh')return {accountKey:params.accountKey,sourceId:params.sourceId,machineId:params.sourceId==='codex-local'?'local':'server',current:loggedIn&&params.accountKey==='codex:a',credential:loggedIn&&['codex:a','codex:b'].includes(params.accountKey),checkedAt:Date.now()/1000};
  if(method==='accounts.deletion.preview'&&params.accountKey==='codex:old')return {tasks:[{id:'offline-task',name:'离线任务',machine:'弃用服务器',root:'~/fixture'}]};
  if(method==='accounts.delete'&&params.accountKey==='codex:old'&&!params.force)return {deleted:false,failedTasks:[{id:'offline-task',name:'离线任务',machine:'弃用服务器',error:'服务器不可达'}]};

  if(method==='codexAuth.inspect')return {profiles:[profile('one'),profile('two')],processes:[],storageMode:'file',version:'fixture'};
  if(method==='codexAuth.login.start'){authDone=false;authCancelled=false;authFailed=false;return {operationId:'login',status:'running'};}
  if(method==='codexAuth.login.status'){
    if(authCancelled)return {operationId:'login',status:'cancelled'};
    if(authFailed)return {operationId:'login',status:'failed',error:'模拟登录失败'};
    if(authDone){const s=await rpc('settings.get');if(!s.accounts.some(a=>a.id==='third')){s.accounts.push({id:'third',provider:'codex',name:'third@example.test',quotaEnabled:true,connections:[{sourceId:'codex-local',profileId:'home:third'}]});await rpc('settings.save',s);}return {operationId:'login',status:'succeeded',accountId:'third'};}
    return {operationId:'login',status:'awaitingAuthorization',authUrl:'https://auth.openai.com/device',userCode:'TEST-CODE'};
  }
  if(method==='codexAuth.login.cancel'){authCancelled=true;return {};}
  if(method==='codexAuth.switch.prepare')return {operationId:'switch',processes:[{pid:123,name:'Codex fixture',canClose:!unknown}]};
  if(method==='codexAuth.switch.commit')return {status:'succeeded',message:'已切换测试账户'};
  if(method==='codexAuth.openUrl')return {};
  if(['quotas.refresh','sources.scan','hosts.sample','prices.list','wakeups.list'].includes(method))return [];
  if(method==='sessions.list')return {sessions:[],warnings:[]};
  if(method.startsWith('network.'))return {status:'ok',sites:[],testedAt:Date.now()/1000};
  return rpc(method,params);
}
function browserFixture(){
  window.setInterval=()=>0;
  window.copied='';Object.defineProperty(navigator,'clipboard',{value:{async writeText(text){window.copied=text;}}});
  window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){
    if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'floating',effectiveMode:'floating',summary:'Idle',sessions:[],panelOpen:true};
    if(command==='engine_call')return window.testRPC(args.method,args.params??{});return {};
  }}};
}
(async()=>{
  const web=path.join(project,'apps/desktop/web'),out=path.join(project,'.local/agent-settings');fs.mkdirSync(out,{recursive:true});
  const server=http.createServer((req,res)=>{if(require('./ui-test-helpers.cjs').serveFont(req,res))return;const file=path.join(web,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
  await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
  try {
    browser=await chromium.launch({headless:true});const page=await browser.newPage({viewport:{width:1180,height:900}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.exposeFunction('testRPC',invoke);await page.addInitScript(browserFixture);await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
    assert.equal(await page.locator('#account option[value="codex:history"]').count(),0);
    const historyTotal=await page.evaluate(()=>state.dashboard.summary.total);assert.equal(historyTotal,100);
    await page.evaluate(()=>{state.page='settings';state.settingsTab='general';state.generalTab='refresh';render();});
    await page.locator('#field-historyShowDeletedAccounts').check();await page.waitForFunction(()=>state.settings.historyShowDeletedAccounts===true&&!state.settingsCommitting);
    await page.evaluate(()=>{state.page='agent';render();});
    assert.equal(await page.locator('#account option[value="codex:history"]').textContent(),'旧账户历史（已删除）');
    await page.locator('#account').selectOption('codex:history');await page.waitForFunction(()=>state.appliedFilters.accountKey==='codex:history'&&!state.dashboardPending);
    assert(calls.some(c=>c.method==='dashboard'&&c.params.accountId==='history'&&c.params.provider==='codex'));
    await page.evaluate(()=>{state.page='settings';state.settingsTab='general';state.generalTab='refresh';render();});
    await page.locator('#field-historyShowDeletedAccounts').uncheck();await page.waitForFunction(()=>state.settings.historyShowDeletedAccounts===false&&!state.settingsCommitting&&state.accountKey==='');
    await page.evaluate(()=>{state.page='agent';render();});
    assert.equal(await page.locator('#account option[value="codex:history"]').count(),0);assert.equal(await page.evaluate(()=>state.dashboard.summary.total),historyTotal);
    await page.evaluate(()=>{state.page='settings';state.settingsTab='hosts';render();});
    const beforeHost=structuredClone((await rpc('settings.get')).hosts.find(h=>h.id==='server'));
    await page.locator('[data-edit="server"]').click();
    await page.locator('#field-authMode').selectOption('password');await page.locator('#field-username').fill('fixture-user');
    failSave=true;
    for(const password of ['fixture-password','replacement-fixture-password']){
      await page.locator('#field-password').fill(password);await page.locator('#editor-form button[type=submit]').click();
      await page.waitForFunction(()=>!document.querySelector('#editor-form button[type=submit]').disabled);
      await page.getByText('模拟保存失败，输入已保留',{exact:true}).waitFor();
      assert.equal(await page.locator('#field-password').inputValue(),password);assert.equal(secrets.size,0);
      assert.deepEqual((await rpc('settings.get')).hosts.find(h=>h.id==='server'),beforeHost);
    }
    assert.equal(calls.filter(c=>c.method==='hosts.credentials.save').length,2);
    assert.equal(calls.filter(c=>c.method==='hosts.credentials.delete').length,2);
    await page.locator('#cancel-editor').click();await page.locator('#editor-confirm-discard').click();
    assert.equal(secrets.size,0);failSave=false;
    await page.locator('[data-edit="server"]').click();await page.locator('#field-authMode').selectOption('password');
    await page.locator('#field-username').fill('fixture-user');await page.locator('#field-password').fill('committed-fixture-password');
    await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.getElementById('editor').open);
    const passwordHost=(await rpc('settings.get')).hosts.find(h=>h.id==='server');
    assert.equal(secrets.get(passwordHost.passwordRef),'committed-fixture-password');assert.equal(secrets.size,1);
    assert(!JSON.stringify(await rpc('settings.get')).includes('committed-fixture-password'));
    await page.locator('[data-edit="server"]').click();await page.locator('#field-password').fill('hidden-password-input');
    await page.locator('#field-authMode').selectOption('ssh');await page.locator('#editor-form button[type=submit]').click();
    await page.waitForFunction(()=>!document.getElementById('editor').open);
    assert.equal(calls.filter(c=>c.method==='hosts.credentials.save').length,3);assert.equal(secrets.size,1);
    assert.equal((await rpc('settings.get')).hosts.find(h=>h.id==='server').passwordRef,passwordHost.passwordRef);
    // A second window can change other fields while this editor remains open.
    await page.locator('[data-edit="server"]').click();await page.locator('#field-port').fill('2222');
    const hostBase=await rpc('settings.get'),concurrent=structuredClone(hostBase);
    concurrent.hosts.find(h=>h.id==='server').name='外部窗口更新';concurrent.refreshSeconds=120;
    await rpc('settings.patch',{base:hostBase,settings:concurrent});
    await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.getElementById('editor').open);
    const mergedHost=(await rpc('settings.get')).hosts.find(h=>h.id==='server');
    assert.equal(mergedHost.name,'外部窗口更新');assert.equal(mergedHost.port,2222);assert.equal((await rpc('settings.get')).refreshSeconds,120);
    await page.evaluate(()=>{state.page='settings';state.settingsTab='sources';render();});
    assert.equal(await page.locator('[data-settings-agent]').count(),await page.evaluate(()=>AieyesContract.agentProviders.length));assert.equal(await page.locator('[data-agent-enable]').count(),1);
    assert.equal(await page.locator('#settings-save-all').count(),0);
    await page.locator('[data-settings-agent=claude]').click();await page.locator('[data-agent-enable="claude"]').check();await page.waitForFunction(()=>state.settings.agents.find(a=>a.provider==='claude').enabled);
    await page.locator('#agent-section-machines > summary').click();
    await page.locator('[data-agent-machine="claude"][value="server"]').check();await page.waitForFunction(()=>state.settings.agents.find(a=>a.provider==='claude').machineIds.length===2);
    // Server switches are independent, and hidden selections survive edits and reloads.
    saveDelay=100; // Exercise saves that outlive Playwright's click completion.
    const showServers=()=>page.evaluate(()=>{state.settingsTab='hosts';render();});
    const showClaude=()=>page.evaluate(()=>{state.settingsTab='sources';state.settingsAgent='claude';render();});
    await showServers();
    assert.equal(await page.locator('[data-device-collection][role=switch]').count(),2);
    await page.locator('[data-device-collection="server"]').uncheck();
    await page.waitForFunction(()=>!state.settings.hosts.find(h=>h.id==='server').enabled&&!state.settingsSaving&&!state.settingsCommitting);
    assert.equal(await page.evaluate(()=>monitoredHosts().find(h=>h.id==='server').enabled),true);
    await showClaude();assert.equal(await page.locator('[data-agent-machine][value="server"]').count(),1,'paused machines remain visible with a recovery action');assert(await page.locator('[data-agent-machine][value="server"]').isChecked());
    await page.locator('[data-agent-machine="claude"][value="local"]').uncheck();
    await page.waitForFunction(()=>!state.settingsSaving);
    assert.deepEqual((await rpc('settings.get')).agents.find(a=>a.provider==='claude').machineIds,['server']);
    await page.locator('[data-agent-machine="claude"][value="local"]').check();await page.waitForFunction(()=>!state.settingsSaving);
    await showServers();await page.locator('[data-device-collection="local"]').uncheck();
    await page.waitForFunction(()=>state.settings.localDeviceEnabled===false&&!state.settingsSaving&&!state.settingsCommitting);
    assert.equal(await page.evaluate(()=>monitoredHosts().find(h=>h.id==='local').enabled),true);
    await showClaude();assert.equal(await page.locator('[data-agent-machine]').count(),2);
    await page.getByText('暂无可采集机器，请在服务器设置开启 Agent 采集。',{exact:true}).waitFor();
    await page.reload();await page.waitForFunction(()=>typeof state!=='undefined'&&state.settings&&!state.busy);
    await page.evaluate(()=>{state.page='settings';state.settingsTab='hosts';render();});
    assert.equal(await page.locator('[data-device-collection="server"]').isChecked(),false);
    for(const id of ['local','server']){
      await page.locator(`[data-device-collection="${id}"]`).check();
      await page.waitForFunction(id=>!state.settingsSaving&&!state.settingsCommitting&&(id==='local'?state.settings.localDeviceEnabled:state.settings.hosts.find(h=>h.id===id).enabled),id);
      await page.locator(`[data-enable="${id}"]`).uncheck();
      await page.waitForFunction(id=>!state.settingsSaving&&!state.settingsCommitting&&!monitoredHosts().find(h=>h.id===id).enabled,id);
      assert.equal(await page.locator(`[data-device-collection="${id}"]`).isChecked(),true);
      await page.locator(`[data-enable="${id}"]`).check();
      await page.waitForFunction(id=>!state.settingsSaving&&!state.settingsCommitting&&monitoredHosts().find(h=>h.id===id).enabled,id);
    }
    failSave=true;await page.locator('[data-device-collection="server"]').click();
    await page.waitForFunction(()=>!state.settingsSaving&&!state.settingsCommitting&&!document.querySelector('[data-device-collection="server"]').disabled);
    assert.equal(await page.locator('[data-device-collection="server"]').isChecked(),true);failSave=false;saveDelay=0;
    await showClaude();
    assert.equal(await page.locator('[data-agent-machine="claude"][value="server"]').isChecked(),true);
    assert.equal(await page.locator('[data-agent-machine="claude"][value="local"]').isChecked(),true);
    await page.screenshot({path:path.join(out,'agents.png'),fullPage:true});
    await page.locator('[data-settings-agent=codex]').click();await page.locator('#agent-section-machines > summary').click();await page.locator('[data-machine-directories="codex"][data-machine="local"]').click();await page.locator('[data-source-advanced="codex-local"]').click();
    assert(await page.locator('#field-hostId').isDisabled());assert.equal(await page.locator('#field-path').getAttribute('readonly'),null);assert.equal(await page.getByText('关联账户与限额',{exact:true}).count(),0);assert.equal(await page.locator('#field-quotaPreCommand').count(),0);
    await page.locator('#field-codexBinary').fill('codex-fixture');failSave=true;await page.locator('#editor-form button[type=submit]').click();await page.getByText('模拟保存失败，输入已保留',{exact:true}).waitFor();assert.equal(await page.locator('#field-codexBinary').inputValue(),'codex-fixture');failSave=false;
    await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.getElementById('editor').open);assert.equal((await rpc('settings.get')).sources[0].codexBinary,'codex-fixture');
    await page.locator('#directories-back').click();await page.locator('[data-settings-agent=claude]').click();await page.locator('[data-machine-directories="claude"][data-machine="local"]').click();
    await page.locator('[data-source-add]').click();await page.locator('#field-path').waitFor();assert.equal(await page.locator('#field-path').inputValue(),claudeRoot);await page.locator('#field-path').fill('~/fixture-secondary');await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.getElementById('editor').open);
    assert.equal(await page.locator('[data-source-remove]').count(),2);const extra=(await rpc('settings.get')).sources.find(s=>s.path==='~/fixture-secondary');
    await page.locator('[data-source-remove="'+extra.id+'"]').click();await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.getElementById('editor').open);assert.equal(await page.locator('[data-source-remove]').count(),1);
    await page.locator('[data-settings-tab="integrations"]').click();await page.locator('[data-settings-agent=codex]').click();assert.equal(await page.locator('[data-account-edit]').count(),2);
    assert.equal(await page.locator('#account-add').count(),0);await page.locator('[data-settings-agent=claude]').click();await page.locator('[data-account-add="claude"]').click();assert.equal(await page.locator('#field-provider').inputValue(),'claude');assert.equal(await page.locator('#field-name').count(),0);for(const checkbox of await page.locator('[name=sourceIds]').all())await checkbox.check();await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.getElementById('editor').open);
    const connected=(await rpc('settings.get')).accounts.find(a=>a.name==='Claude Code 账户');assert.equal(connected.connections.length,2);
    await page.screenshot({path:path.join(out,'accounts.png'),fullPage:true});
    assert.equal(await page.locator('.account-agent-group').count(),1);
    for(const group of await page.locator('.account-agent-group').all()){const heading=await group.locator('h2').boundingBox(),button=await group.locator('[data-account-add]').boundingBox();assert(button.x>heading.x+heading.width);}
    await page.locator('[data-settings-agent=codex]').click();await page.waitForFunction(()=>!AieyesAgentSettings.statusBusy);assert.equal(await page.locator('#account-status-refresh').count(),0);assert.equal(await page.locator('[data-device-manage][data-intent="login"]').count(),1);assert.equal(await page.locator('[data-device-manage][data-intent="switch"]').count(),1);
    await page.locator('[data-settings-agent=claude]').click();await page.locator('[data-account-edit="'+connected.provider+':'+connected.id+'"]').click();await page.getByText('前置命令',{exact:true}).click();await page.locator('[data-command-field="server"]').fill('export ACCOUNT_FIXTURE=claude');await page.locator('[data-command-field="server"]').blur();await page.waitForFunction(()=>state.settings.accounts.some(a=>a.deviceSettings?.some(d=>d.preCommand==='export ACCOUNT_FIXTURE=claude')));
    assert.equal((await rpc('settings.get')).accounts.find(a=>a.id===connected.id).deviceSettings[0].preCommand,'export ACCOUNT_FIXTURE=claude');
    assert.equal(await page.locator('[data-account-reset=unexpected]').inputValue(),'inherit');
    await page.locator('[data-account-reset=unexpected]').selectOption('off');
    await page.waitForFunction(()=>state.settings.accounts.some(a=>a.quotaResetNotifications?.unexpected===false));
    await page.locator('[data-account-reset=natural]').selectOption('on');
    await page.waitForFunction(()=>state.settings.accounts.some(a=>a.quotaResetNotifications?.natural===true));
    assert.deepEqual((await rpc('settings.get')).accounts.find(a=>a.id===connected.id).quotaResetNotifications,{unexpected:false,natural:true});
    await page.locator('[data-account-reset=unexpected]').selectOption('inherit');
    await page.waitForFunction(()=>state.settings.accounts.some(a=>a.quotaResetNotifications?.unexpected===null&&a.quotaResetNotifications?.natural===true));
    await page.locator('#account-interval').fill('60');await page.locator('#account-interval').blur();await page.waitForFunction(()=>state.settings.accounts.some(a=>a.quotaRefreshSeconds===60));
    failSave=true;await page.locator('#account-name').fill('保留失败输入');await page.locator('#account-name').blur();await page.locator('[data-account-retry]').waitFor();assert.equal(await page.locator('#account-name').inputValue(),'保留失败输入');failSave=false;await page.locator('[data-account-retry]').click();await page.waitForFunction(()=>state.settings.accounts.some(a=>a.name==='保留失败输入'));
    await page.locator('#account-interval').fill('4');await page.locator('#account-interval').blur();await page.getByText('查询间隔范围为 5–86400 秒',{exact:false}).first().waitFor();assert.equal((await rpc('settings.get')).accounts.find(a=>a.id===connected.id).quotaRefreshSeconds,60);
    await page.locator('#account-interval').fill('5');await page.locator('#account-interval').blur();await page.waitForFunction(()=>state.settings.accounts.some(a=>a.quotaRefreshSeconds===5));
    await page.locator('#account-interval').fill('120');await page.locator('#account-interval').blur();await page.waitForFunction(()=>state.settings.accounts.some(a=>a.quotaRefreshSeconds===120));
    await page.evaluate(()=>{
      const quota=document.querySelector('[data-account-quota]');quota.checked=false;quota.dispatchEvent(new Event('change'));
      const interval=document.querySelector('#account-interval');interval.value='180';interval.dispatchEvent(new Event('input'));interval.dispatchEvent(new Event('change'));
      quota.checked=true;quota.dispatchEvent(new Event('change'));
    });await page.waitForFunction(()=>state.settings.accounts.some(a=>a.quotaRefreshSeconds===180&&a.quotaEnabled)&&!state.settingsSaving);
    await page.locator('#account-detail-back').click();
    await page.locator('[data-settings-agent=codex]').click();await page.locator('[data-account-edit="codex:a"]').click();assert.equal(await page.locator('[data-account-login]').count(),0);await page.getByText('正在使用',{exact:true}).waitFor();await page.screenshot({path:path.join(out,'account-devices.png'),fullPage:true});
    await page.locator('#account-detail-back').click();await page.locator('.machine-menu > summary').first().click();await page.locator('[data-device-manage="codex-local"][data-intent="switch"]').click();await page.locator('[data-auth="switch"]').waitFor();unknown=true;await page.locator('[data-auth="switch"]').click();assert(await page.locator('[data-auth="commit"]').isDisabled());await page.locator('[data-auth="cancelSwitch"]').click();unknown=false;
    loggedIn=false;await page.locator('#accounts-back').click();await page.locator('.machine-menu > summary').first().click();await page.locator('[data-device-manage="codex-local"][data-intent="login"]').click();await page.locator('[data-auth="login"]').waitFor();
    await page.locator('#accounts-back').click();const beforeLogin=(await rpc('settings.get')).accounts.map(a=>a.id);await page.locator('[data-account-add="codex"]').click();await page.locator('[data-auth="login"]').waitFor();assert.equal(await page.locator('#auth-account').count(),0);assert.equal(await page.locator('#field-name').count(),0);assert.equal(await page.locator('#editor[open]').count(),0);assert.deepEqual((await rpc('settings.get')).accounts.map(a=>a.id),beforeLogin);await page.locator('[data-auth="login"]').click();assert.equal(await page.locator('#field-name').count(),0);await page.locator('#editor-form button[type=submit]').click();assert.equal(calls.filter(c=>c.method==='codexAuth.login.start').at(-1).params.name,undefined);await page.locator('[data-auth="copyUrl"]').waitFor();await page.locator('[data-auth="copyUrl"]').click();assert.equal(await page.evaluate(()=>window.copied),'https://auth.openai.com/device');await page.locator('[data-auth="copyCode"]').click();assert.equal(await page.evaluate(()=>window.copied),'TEST-CODE');
    await page.screenshot({path:path.join(out,'login.png'),fullPage:true});authDone=true;loggedIn=true;await page.getByText('登录成功，已启用此账户的额度查询。',{exact:true}).waitFor();await page.locator('#accounts-back').click();await page.locator('[data-account-edit="codex:third"]').waitFor();assert.equal((await rpc('settings.get')).accounts.find(a=>a.id==='third').name,'third@example.test');assert.equal(calls.filter(c=>c.method==='codexAuth.profiles.bind').length,0);
    await page.locator('[data-account-edit="codex:b"]').click();assert.equal(await page.locator('[data-account-login][data-intent="login"]').count(),0);await page.getByText('已登录设备 1/1 · 使用中设备 0/1',{exact:true}).waitFor();await page.getByText('已登录 · 未使用',{exact:true}).waitFor();await page.locator('[data-account-login="codex:b"][data-intent="switch"]').first().click();await page.locator('[data-auth="commit"]').waitFor();assert.equal(calls.filter(c=>c.method==='codexAuth.switch.prepare').at(-1).params.profileId,'two');assert.equal(await page.locator('[data-auth="login"]').count(),0);await page.locator('[data-auth="cancelSwitch"]').click();await page.locator('#accounts-back').click();await page.locator('#account-detail-back').click();
    // Archived accounts stay archived; restoring is explicit and immediate.
    assert((await rpc('settings.get')).accounts.find(a=>a.id==='old').archived);
    await page.getByText('已归档账户',{exact:true}).click();await page.locator('[data-account-restore="codex:old"]').click();await page.locator('[data-account-edit="codex:old"]').waitFor();
    const archiveBase=await rpc('settings.get'),archiveNext=structuredClone(archiveBase);archiveNext.accounts.find(a=>a.id==='old').archived=true;const archived=await rpc('settings.patch',{base:archiveBase,settings:archiveNext});await page.evaluate(s=>{window.AieyesAgentSettings.accept(s);renderSettings(false);},archived);
    await page.getByText('已归档账户',{exact:true}).click();await page.locator('[data-account-delete="codex:old"]').click();await page.locator('#editor-form button[type=submit]').click();await page.getByText('部分任务清理失败，账户仍保留；可重试或勾选强制删除。',{exact:true}).waitFor();assert((await rpc('settings.get')).accounts.some(a=>a.id==='old'));
    await page.screenshot({path:path.join(out,'delete-account.png'),fullPage:true});await page.locator('[name=force]').check();await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.getElementById('editor').open);assert(!(await rpc('settings.get')).accounts.some(a=>a.id==='old'));assert(!(await rpc('settings.get')).deletedAccounts.some(a=>a.id==='old')); // Empty deleted accounts never return as history filters.
    // Adding a real login never creates an empty account; failures and cancellation leave the list intact.
    const beforeCancelled=(await rpc('settings.get')).accounts.map(a=>a.id);
    await page.locator('[data-account-add="codex"]').click();await page.locator('[data-auth="login"]').waitFor();assert.equal(await page.locator('#auth-account').count(),0);assert.equal(await page.locator('#editor[open]').count(),0);
    await page.locator('[data-auth="login"]').click();await page.locator('#editor-form button[type=submit]').click();await page.locator('[data-auth="cancelLogin"]').waitFor();authFailed=true;await page.getByText('模拟登录失败',{exact:false}).first().waitFor();assert.deepEqual((await rpc('settings.get')).accounts.map(a=>a.id),beforeCancelled);
    await page.locator('[data-auth="login"]').click();await page.locator('#editor-form button[type=submit]').click();await page.locator('[data-auth="cancelLogin"]').click();await page.locator('[data-auth="cancelLogin"]').waitFor({state:'detached'});assert.deepEqual((await rpc('settings.get')).accounts.map(a=>a.id),beforeCancelled);
    assert.equal(calls.filter(c=>c.method==='accounts.create').length,0);await page.locator('#accounts-back').click();
    // Global notification preferences use the real configuration merge and survive navigation.
    await page.evaluate(()=>{state.settingsTab='general';state.generalTab='notifications';renderSettings();});
    assert.equal(await page.getByRole('checkbox',{name:'提前重置时提醒',exact:true}).isChecked(),true);
    assert.equal(await page.getByRole('checkbox',{name:'到期重置时提醒',exact:true}).isChecked(),false);
    await page.getByRole('checkbox',{name:'提前重置时提醒',exact:true}).uncheck();
    await page.waitForFunction(()=>state.settings.quotaResetNotifications?.unexpected===false&&!state.settingsCommitting);
    await page.getByRole('checkbox',{name:'到期重置时提醒',exact:true}).check();
    await page.waitForFunction(()=>state.settings.quotaResetNotifications?.natural===true&&!state.settingsCommitting);
    assert.deepEqual((await rpc('settings.get')).quotaResetNotifications,{unexpected:false,natural:true});
    await page.locator('[data-general-page=appearance]').click();await page.locator('[data-general-page=notifications]').click();
    assert.equal(await page.getByRole('checkbox',{name:'提前重置时提醒',exact:true}).isChecked(),false);
    assert.equal(await page.getByRole('checkbox',{name:'到期重置时提醒',exact:true}).isChecked(),true);
    await page.screenshot({path:path.join(out,'notifications.png'),fullPage:true});
    // Existing mappings are lazy, editable, and retain the editor on real-core conflicts.
    const mappingBase=await rpc('settings.get'),mappingNext=structuredClone(mappingBase);mappingNext.modelMappings={'fixture-old':'provider/old'};
    const mappingSaved=await rpc('settings.patch',{base:mappingBase,settings:mappingNext});
    await page.evaluate(saved=>{AieyesAgentSettings.accept(saved);state.settingsTab='prices';renderSettings(false);},mappingSaved);
    assert.equal(await page.locator('#existing-mappings').evaluate(el=>el.open),false);assert.equal(await page.locator('[data-mapping-edit]').count(),0);
    await page.locator('#existing-mappings > summary').click();await page.locator('[data-mapping-edit="fixture-old"]').click();
    await page.locator('#field-to').fill('provider/cancelled');await page.locator('#cancel-editor').click();await page.locator('#editor-confirm-discard').click();
    assert.equal((await rpc('settings.get')).modelMappings['fixture-old'],'provider/old');
    await page.locator('[data-mapping-edit="fixture-old"]').click();await page.locator('#field-from').fill('fixture-new');await page.locator('#field-to').fill('provider/new');
    await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.querySelector('#editor').open);
    assert.deepEqual((await rpc('settings.get')).modelMappings,{'fixture-new':'provider/new'});
    if(!await page.locator('#existing-mappings').evaluate(el=>el.open))await page.locator('#existing-mappings > summary').click();
    await page.locator('[data-mapping-edit="fixture-new"]').click();await page.locator('#field-to').fill('provider/draft');
    const conflictBase=await rpc('settings.get'),conflictNext=structuredClone(conflictBase);conflictNext.modelMappings['fixture-new']='provider/external';await rpc('settings.patch',{base:conflictBase,settings:conflictNext});
    await page.locator('#editor-form button[type=submit]').click();await page.locator('#editor-error:visible').waitFor();
    assert.equal(await page.locator('#field-to').inputValue(),'provider/draft');assert.equal((await rpc('settings.get')).modelMappings['fixture-new'],'provider/external');
    await page.locator('#cancel-editor').click();await page.locator('#editor-confirm-discard').click();
    // Unsupported forms are absent, including guidance-only API placeholders.
    for(const provider of ['antigravity','cursor','custom']){
      await page.evaluate(provider=>{state.page='settings';state.settingsTab='integrations';AieyesAgentSettings.navigate({provider});render();},provider);
      assert.equal(await page.locator('#agent-section-api').count(),0,provider);
    }
    for(const provider of ['deepseek','qoder','grok','cursor','codebuddy','pi','omp']){
      await page.evaluate(provider=>{AieyesAgentSettings.navigate({provider});render();},provider);
      assert.equal(await page.locator('#agent-section-valuation').count(),0,provider);
    }
    await page.evaluate(()=>{AieyesAgentSettings.navigate({provider:'deepseek-harness'});render();});
    assert.equal(await page.locator('[data-settings-agent="deepseek"]').count(),1);
    assert.equal(await page.locator('[data-settings-agent="deepseek-harness"]').count(),0);
    assert.equal(await page.locator('[data-agent-section]').getAttribute('data-agent-section'),'deepseek');
    assert.equal(await page.locator('#agent-section-api').count(),1);
    assert.equal(await page.locator('#agent-section-machines').count(),1);
    await page.locator('[data-agent-enable="deepseek"]').check();
    await page.waitForFunction(()=>!state.settingsSaving&&state.settings.agents.filter(a=>a.provider.startsWith('deepseek')).every(a=>a.enabled));
    await page.locator('[data-agent-enable="deepseek"]').uncheck();
    await page.waitForFunction(()=>!state.settingsSaving&&state.settings.agents.filter(a=>a.provider.startsWith('deepseek')).every(a=>!a.enabled));
    await page.evaluate(()=>{AieyesAgentSettings.navigate({provider:'cursor'});render();});
    // Native Cursor observers use flat hooks.json entries and keep user hooks.
    const cursorHome=path.join(root,'.cursor');fs.mkdirSync(cursorHome);fs.writeFileSync(path.join(cursorHome,'hooks.json'),JSON.stringify({version:1,hooks:{preToolUse:[{command:'other-guard'}]}}));
    const cursorSettings=await rpc('settings.get');cursorSettings.sources.push({id:'cursor-quota-ui',provider:'cursor',name:'Cursor quota',path:'',enabled:true,subscription:{mode:'session',credentialRef:'fixture-credential'}});const cursorAgent=cursorSettings.agents.find(a=>a.provider==='cursor');cursorAgent.enabled=true;cursorAgent.machineIds=['local'];await rpc('settings.save',cursorSettings);
    await page.evaluate(async()=>{state.settings=await window.testRPC('settings.get');render();});
    assert.equal(await page.locator('[data-machine-hooks="cursor"][data-machine="local"]').isDisabled(),true);
    cursorSettings.sources.push({id:'cursor-ui',provider:'cursor',name:'Cursor UI',path:cursorHome,enabled:true});await rpc('settings.save',cursorSettings);
    await page.evaluate(async()=>{state.settings=await window.testRPC('settings.get');render();});
    if(!await page.locator('#agent-section-machines').evaluate(el=>el.open))await page.locator('#agent-section-machines > summary').click();
    await page.locator('[data-machine-hooks="cursor"][data-machine="local"]').click();
    await page.getByText(/从用户或项目的 .cursor\/hooks.json 接收状态/).waitFor();
    assert.equal(await page.locator('#session-hooks [data-hook-row]').count(),1);
    await page.locator('#session-hooks [data-hook-repair]').click();await page.locator('#session-hooks [data-hook-label]').getByText('已安装',{exact:true}).waitFor();
    const installedCursor=JSON.parse(fs.readFileSync(path.join(cursorHome,'hooks.json')));assert.equal(installedCursor.hooks.preToolUse[0].command,'other-guard');assert.equal(typeof installedCursor.hooks.preToolUse[1].command,'string');assert.equal(installedCursor.hooks.preToolUse[1].hooks,undefined);
    await page.locator('#session-hooks [data-hook-toggle]').click();await page.locator('#session-hooks [data-hook-label]').getByText('已停用',{exact:true}).waitFor();assert.equal(await page.locator('#session-hooks [data-hook-toggle]').isChecked(),false);
    assert.equal(JSON.parse(fs.readFileSync(path.join(cursorHome,'hooks.json'))).hooks.preToolUse.length,1);
    await page.locator('#session-hooks [data-hook-toggle]').click();await page.locator('#session-hooks [data-hook-label]').getByText('已安装',{exact:true}).waitFor();assert.equal(await page.locator('#session-hooks [data-hook-toggle]').isChecked(),true);
    await page.screenshot({animations:'disabled',path:path.join(out,'cursor-session-hooks.png')});await page.locator('#session-hooks [data-close]').click();
    const zcodeHome=path.join(root,'.zcode');fs.mkdirSync(path.join(zcodeHome,'cli'),{recursive:true});
    const zcodeConfig=path.join(zcodeHome,'cli/config.json'),foreignZcode={hooks:[{type:'process',command:'other-observer',args:['keep']}]};
    fs.writeFileSync(zcodeConfig,JSON.stringify({mcp:{keep:true},hooks:{events:{PreToolUse:[foreignZcode]}}}));
    const zcodeSettings=await rpc('settings.get');zcodeSettings.sources.push({id:'zcode-ui',provider:'zcode',name:'ZCode UI',path:zcodeHome,enabled:true});
    const zcodeAgent=zcodeSettings.agents.find(a=>a.provider==='zcode');zcodeAgent.enabled=true;zcodeAgent.machineIds=['local'];await rpc('settings.save',zcodeSettings);
    await page.evaluate(async()=>{state.settings=await api('settings.get');AieyesAgentSettings.navigate({provider:'zcode'});render();});
    if(!await page.locator('#agent-section-machines').evaluate(el=>el.open))await page.locator('#agent-section-machines > summary').click();
    await page.locator('[data-machine-hooks="zcode"][data-machine="local"]').click();await page.getByText(/实时状态由 ZCode 原生监测提供/).waitFor();
    await page.locator('#session-hooks [data-hook-repair]').click();await page.locator('#session-hooks [data-hook-label]').getByText('已安装',{exact:true}).waitFor();
    let installedZcode=JSON.parse(fs.readFileSync(zcodeConfig));assert.equal(installedZcode.hooks.enabled,true);assert.deepEqual(installedZcode.hooks.events.PreToolUse[0],foreignZcode);assert.equal(installedZcode.hooks.events.Stop[0].hooks[0].type,'process');assert.equal(installedZcode.hooks.events.Stop[0].matcher,undefined);
    await page.locator('#session-hooks [data-hook-toggle]').click();await page.locator('#session-hooks [data-hook-label]').getByText('已停用',{exact:true}).waitFor();assert.equal(await page.locator('#session-hooks [data-hook-toggle]').isChecked(),false);
    installedZcode=JSON.parse(fs.readFileSync(zcodeConfig));assert.deepEqual(installedZcode.hooks.events.PreToolUse,[foreignZcode]);assert.equal(installedZcode.mcp.keep,true);
    await page.locator('#session-hooks [data-hook-toggle]').click();await page.locator('#session-hooks [data-hook-label]').getByText('已安装',{exact:true}).waitFor();
    await page.screenshot({animations:'disabled',path:path.join(out,'zcode-session-hooks.png')});await page.locator('#session-hooks [data-close]').click();
    // Two normal accounts coexist in the compact floating panel renderer.
    const panel=await browser.newPage({viewport:{width:450,height:900}});await panel.exposeFunction('testRPC',invoke);await panel.addInitScript(browserFixture);panel.on('pageerror',e=>errors.push(String(e)));await panel.goto(`http://127.0.0.1:${server.address().port}/floating.html`);await panel.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);assert(await panel.getByText('个人 Codex',{exact:true}).count());assert(await panel.getByText('工作 Codex',{exact:true}).count());await panel.screenshot({path:path.join(out,'panel.png'),fullPage:true});
    for(const theme of ['light','dark'])for(const accent of ['indigo','blue','teal','purple']){
      await page.evaluate(({theme,accent})=>AieyesTheme.apply({theme,accent}),{theme,accent});
      const colors=await page.evaluate(()=>{const css=getComputedStyle(document.documentElement);return {accent:css.getPropertyValue('--accent').trim(),balance:css.getPropertyValue('--balance-blue').trim()};});assert.equal(colors.balance,colors.accent);
    }
    assert.deepEqual(errors,[]);console.log('Agents/account UI: real-core immediate save, machines, directory add/remove, retained failed drafts, independent intervals, retry, multi-machine account, auth copy/auto-connect, archive and multi-account panel passed');
  } finally {if(browser)await browser.close();server.close();core.stdin.end();if(core.exitCode===null)await new Promise(resolve=>core.once('exit',resolve));fs.rmSync(root,{recursive:true,force:true});}
})().catch(error=>{console.error(error);core.kill();process.exitCode=1;});
