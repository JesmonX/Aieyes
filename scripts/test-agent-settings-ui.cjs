/* Configuration calls use the real core and a disposable database; auth is simulated. */
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),os=require('node:os'),http=require('node:http'),readline=require('node:readline');
const {spawn,execFileSync}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const project=path.resolve(__dirname,'..'),root=fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-settings-'));
const fixture={version:2,sources:[{id:'codex-local',name:'Codex · 本机',provider:'codex',path:root+'/codex',accountId:'',enabled:true,codexHomeId:'home'}],accounts:[{id:'a',name:'个人 Codex',provider:'codex',quotaEnabled:true,quotaProfileId:'home:one'},{id:'b',name:'工作 Codex',provider:'codex',quotaEnabled:true,quotaProfileId:'home:two'},{id:'old',name:'归档账户',provider:'codex',quotaEnabled:true,archived:true}],hosts:[{id:'server',name:'测试服务器',target:'fixture.invalid',enabled:true}],modelMappings:{}};
execFileSync('python3',['-c','import sqlite3,json,sys; c=sqlite3.connect(sys.argv[1]); c.execute("CREATE TABLE kv(key TEXT PRIMARY KEY,value TEXT NOT NULL)"); c.execute("INSERT INTO kv VALUES(?,?)",("settings",sys.argv[2])); c.commit()',path.join(root,'aieyes.sqlite'),JSON.stringify(fixture)]);
const core=spawn(path.join(project,'target/debug/aieyes-core'),['--data-dir',root],{stdio:['pipe','pipe','pipe']});
let counter=0;const pending=new Map();let stderr='';core.stderr.on('data',b=>stderr+=b);
readline.createInterface({input:core.stdout}).on('line',line=>{const r=JSON.parse(line),p=pending.get(r.id);if(!p)return;pending.delete(r.id);if(r.error)p.reject(new Error(r.error.message));else p.resolve(r.result);});
core.on('exit',()=>{for(const p of pending.values())p.reject(new Error('Core exited: '+stderr));pending.clear();});
function rpc(method,params={}){return new Promise((resolve,reject)=>{const id=++counter;pending.set(id,{resolve,reject});core.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');});}
const profile=id=>({id,name:id==='one'?'个人 Codex':'工作 Codex',current:id==='one',identity:{key:id,workspace:'workspace',email:id+'@example.test',plan:'plus'}});
let authDone=false,authCancelled=false,unknown=false,failSave=false,loggedIn=true,authFailed=false;const calls=[];
async function invoke(method,params){
  calls.push({method,params});
  if(['settings.patch','sources.configure'].includes(method)&&failSave)throw new Error('模拟保存失败，输入已保留');
  if(method==='accounts.status.refresh')return {accountKey:params.accountKey,sourceId:params.sourceId,machineId:params.sourceId==='codex-local'?'local':'server',current:loggedIn&&params.accountKey==='codex:a',credential:loggedIn&&['codex:a','codex:b'].includes(params.accountKey),checkedAt:Date.now()/1000};
  if(method==='accounts.deletion.preview'&&params.accountKey==='codex:old')return {tasks:[{id:'offline-task',name:'离线任务',machine:'弃用服务器',root:'~/fixture'}]};
  if(method==='accounts.delete'&&params.accountKey==='codex:old'&&!params.force)return {deleted:false,failedTasks:[{id:'offline-task',name:'离线任务',machine:'弃用服务器',error:'服务器不可达'}]};

  if(method==='codexAuth.inspect')return {profiles:[profile('one'),profile('two')],processes:[],storageMode:'file',version:'fixture'};
  if(method==='codexAuth.login.start'){authDone=false;authCancelled=false;authFailed=false;return {operationId:'login',status:'running'};}
  if(method==='codexAuth.login.status'){
    if(authCancelled)return {operationId:'login',status:'cancelled'};
    if(authFailed)return {operationId:'login',status:'failed',error:'模拟登录失败'};
    if(authDone){const s=await rpc('settings.get');if(!s.accounts.some(a=>a.id==='third')){s.accounts.push({id:'third',provider:'codex',name:'third@example.test',quotaEnabled:true,quotaProfileId:'home:third'});await rpc('settings.save',s);}return {operationId:'login',status:'succeeded',accountId:'third'};}
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
    await page.evaluate(()=>{state.page='settings';state.settingsTab='sources';render();});
    assert.equal(await page.locator('[data-agent-enable]').count(),5);
    assert.equal(await page.locator('#settings-save-all').count(),0);
    await page.locator('[data-agent-enable="claude"]').check();await page.waitForFunction(()=>state.settings.agents.find(a=>a.provider==='claude').enabled);
    await page.locator('[data-agent-machine="claude"][value="server"]').check();await page.waitForFunction(()=>state.settings.agents.find(a=>a.provider==='claude').machineIds.length===2);
    await page.screenshot({path:path.join(out,'agents.png'),fullPage:true});
    await page.locator('[data-machine-directories="codex"][data-machine="local"]').click();await page.locator('[data-source-advanced="codex-local"]').click();
    assert(await page.locator('#field-hostId').isDisabled());assert.equal(await page.locator('#field-path').getAttribute('readonly'),null);assert.equal(await page.getByText('关联账户与限额',{exact:true}).count(),0);assert.equal(await page.locator('#field-quotaPreCommand').count(),0);
    await page.locator('#field-codexBinary').fill('codex-fixture');failSave=true;await page.locator('#editor-form button[type=submit]').click();await page.getByText('模拟保存失败，输入已保留',{exact:true}).waitFor();assert.equal(await page.locator('#field-codexBinary').inputValue(),'codex-fixture');failSave=false;
    await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.getElementById('editor').open);assert.equal((await rpc('settings.get')).sources[0].codexBinary,'codex-fixture');
    await page.locator('#directories-back').click();await page.locator('[data-machine-directories="claude"][data-machine="local"]').click();
    await page.locator('[data-source-add]').click();await page.locator('#field-path').fill('~/fixture-secondary');await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.getElementById('editor').open);
    assert.equal(await page.locator('[data-source-remove]').count(),2);const extra=(await rpc('settings.get')).sources.find(s=>s.path==='~/fixture-secondary');
    await page.locator('[data-source-remove="'+extra.id+'"]').click();await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.getElementById('editor').open);assert.equal(await page.locator('[data-source-remove]').count(),1);
    await page.locator('[data-settings-tab="accounts"]').click();assert.equal(await page.locator('[data-account-edit]').count(),2);
    assert.equal(await page.locator('#account-add').count(),0);await page.locator('[data-account-add="claude"]').click();assert.equal(await page.locator('#field-provider').inputValue(),'claude');assert.equal(await page.locator('#field-name').count(),0);for(const checkbox of await page.locator('[name=sourceIds]').all())await checkbox.check();await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.getElementById('editor').open);
    const connected=(await rpc('settings.get')).accounts.find(a=>a.name==='Claude Code 账户');assert.equal(connected.connections.length,2);
    await page.screenshot({path:path.join(out,'accounts.png'),fullPage:true});
    assert.equal(await page.locator('.account-agent-group').count(),2);
    for(const group of await page.locator('.account-agent-group').all()){const heading=await group.locator('h2').boundingBox(),button=await group.locator('[data-account-add]').boundingBox();assert(button.x>heading.x+heading.width);}
    await page.waitForFunction(()=>!document.getElementById('account-status-refresh').disabled);assert.equal(await page.locator('[data-device-manage][data-intent="login"]').count(),0);assert.equal(await page.locator('[data-device-manage][data-intent="switch"]').count(),1);
    await page.locator('[data-account-edit="'+connected.provider+':'+connected.id+'"]').click();await page.getByText('前置命令',{exact:true}).click();await page.locator('[data-command-field="server"]').fill('export ACCOUNT_FIXTURE=claude');await page.locator('[data-command-field="server"]').blur();await page.waitForFunction(()=>state.settings.accounts.some(a=>a.deviceSettings?.some(d=>d.preCommand==='export ACCOUNT_FIXTURE=claude')));
    assert.equal((await rpc('settings.get')).accounts.find(a=>a.id===connected.id).deviceSettings[0].preCommand,'export ACCOUNT_FIXTURE=claude');
    await page.locator('#account-interval').fill('60');await page.locator('#account-interval').blur();await page.waitForFunction(()=>state.settings.accounts.some(a=>a.quotaRefreshSeconds===60));
    failSave=true;await page.locator('#account-name').fill('保留失败输入');await page.locator('#account-name').blur();await page.locator('[data-account-retry]').waitFor();assert.equal(await page.locator('#account-name').inputValue(),'保留失败输入');failSave=false;await page.locator('[data-account-retry]').click();await page.waitForFunction(()=>state.settings.accounts.some(a=>a.name==='保留失败输入'));
    await page.locator('#account-interval').fill('5');await page.locator('#account-interval').blur();await page.getByText('查询间隔范围为 30–86400 秒',{exact:false}).first().waitFor();assert.equal((await rpc('settings.get')).accounts.find(a=>a.id===connected.id).quotaRefreshSeconds,60);
    await page.locator('#account-interval').fill('120');await page.locator('#account-interval').blur();await page.waitForFunction(()=>state.settings.accounts.some(a=>a.quotaRefreshSeconds===120));
    await page.evaluate(()=>{
      const quota=document.querySelector('[data-account-quota]');quota.checked=false;quota.dispatchEvent(new Event('change'));
      const interval=document.querySelector('#account-interval');interval.value='180';interval.dispatchEvent(new Event('input'));interval.dispatchEvent(new Event('change'));
      quota.checked=true;quota.dispatchEvent(new Event('change'));
    });await page.waitForFunction(()=>state.settings.accounts.some(a=>a.quotaRefreshSeconds===180&&a.quotaEnabled)&&!state.settingsSaving);
    await page.locator('#account-detail-back').click();
    await page.locator('[data-account-edit="codex:a"]').click();assert.equal(await page.locator('[data-account-login]').count(),0);await page.getByText('正在使用',{exact:true}).waitFor();await page.screenshot({path:path.join(out,'account-devices.png'),fullPage:true});
    await page.locator('#account-detail-back').click();await page.locator('[data-device-manage="codex-local"][data-intent="switch"]').click();await page.locator('[data-auth="switch"]').waitFor();unknown=true;await page.locator('[data-auth="switch"]').click();assert(await page.locator('[data-auth="commit"]').isDisabled());await page.locator('[data-auth="cancelSwitch"]').click();unknown=false;
    loggedIn=false;await page.locator('#accounts-back').click();await page.waitForFunction(()=>!document.getElementById('account-status-refresh').disabled);assert.equal(await page.locator('[data-device-manage][data-intent="switch"]').count(),0);await page.locator('[data-device-manage="codex-local"][data-intent="login"]').click();await page.locator('[data-auth="login"]').waitFor();
    await page.locator('#accounts-back').click();const beforeLogin=(await rpc('settings.get')).accounts.map(a=>a.id);await page.locator('[data-account-add="codex"]').click();await page.locator('[data-auth="login"]').waitFor();assert.equal(await page.locator('#auth-account').count(),0);assert.equal(await page.locator('#field-name').count(),0);assert.equal(await page.locator('#editor[open]').count(),0);assert.deepEqual((await rpc('settings.get')).accounts.map(a=>a.id),beforeLogin);await page.locator('[data-auth="login"]').click();assert.equal(await page.locator('#field-name').count(),0);await page.locator('#editor-form button[type=submit]').click();assert.equal(calls.filter(c=>c.method==='codexAuth.login.start').at(-1).params.name,undefined);await page.locator('[data-auth="copyUrl"]').waitFor();await page.locator('[data-auth="copyUrl"]').click();assert.equal(await page.evaluate(()=>window.copied),'https://auth.openai.com/device');await page.locator('[data-auth="copyCode"]').click();assert.equal(await page.evaluate(()=>window.copied),'TEST-CODE');
    await page.screenshot({path:path.join(out,'login.png'),fullPage:true});authDone=true;loggedIn=true;await page.getByText('登录成功，已启用此账号的额度查询。',{exact:true}).waitFor();await page.locator('#accounts-back').click();await page.locator('[data-account-edit="codex:third"]').waitFor();assert.equal((await rpc('settings.get')).accounts.find(a=>a.id==='third').name,'third@example.test');assert.equal(calls.filter(c=>c.method==='codexAuth.profiles.bind').length,0);
    await page.locator('[data-account-edit="codex:b"]').click();assert.equal(await page.locator('[data-account-login][data-intent="login"]').count(),0);await page.getByText('已登录 1/1 · 使用中 0/1',{exact:true}).waitFor();await page.getByText('已登录 · 未使用',{exact:true}).waitFor();await page.locator('[data-account-login="codex:b"][data-intent="switch"]').first().click();await page.locator('[data-auth="commit"]').waitFor();assert.equal(calls.filter(c=>c.method==='codexAuth.switch.prepare').at(-1).params.profileId,'two');assert.equal(await page.locator('[data-auth="login"]').count(),0);await page.locator('[data-auth="cancelSwitch"]').click();await page.locator('#accounts-back').click();await page.locator('#account-detail-back').click();
    // Archived accounts stay archived; restoring is explicit and immediate.
    assert((await rpc('settings.get')).accounts.find(a=>a.id==='old').archived);
    await page.getByText('已归档账户',{exact:true}).click();await page.locator('[data-account-restore="codex:old"]').click();await page.locator('[data-account-edit="codex:old"]').waitFor();
    const archiveBase=await rpc('settings.get'),archiveNext=structuredClone(archiveBase);archiveNext.accounts.find(a=>a.id==='old').archived=true;const archived=await rpc('settings.patch',{base:archiveBase,settings:archiveNext});await page.evaluate(s=>{window.AieyesAgentSettings.accept(s);renderSettings(false);},archived);
    await page.getByText('已归档账户',{exact:true}).click();await page.locator('[data-account-delete="codex:old"]').click();await page.locator('#editor-form button[type=submit]').click();await page.getByText('部分任务清理失败，账户仍保留；可重试或勾选强制删除。',{exact:true}).waitFor();assert((await rpc('settings.get')).accounts.some(a=>a.id==='old'));
    await page.screenshot({path:path.join(out,'delete-account.png'),fullPage:true});await page.locator('[name=force]').check();await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.getElementById('editor').open);assert(!(await rpc('settings.get')).accounts.some(a=>a.id==='old'));assert((await rpc('settings.get')).deletedAccounts.some(a=>a.id==='old'));
    // Adding a real login never creates an empty account; failures and cancellation leave the list intact.
    const beforeCancelled=(await rpc('settings.get')).accounts.map(a=>a.id);
    await page.locator('[data-account-add="codex"]').click();await page.locator('[data-auth="login"]').waitFor();assert.equal(await page.locator('#auth-account').count(),0);assert.equal(await page.locator('#editor[open]').count(),0);
    await page.locator('[data-auth="login"]').click();await page.locator('#editor-form button[type=submit]').click();await page.locator('[data-auth="cancelLogin"]').waitFor();authFailed=true;await page.getByText('模拟登录失败',{exact:false}).first().waitFor();assert.deepEqual((await rpc('settings.get')).accounts.map(a=>a.id),beforeCancelled);
    await page.locator('[data-auth="login"]').click();await page.locator('#editor-form button[type=submit]').click();await page.locator('[data-auth="cancelLogin"]').click();await page.locator('[data-auth="cancelLogin"]').waitFor({state:'detached'});assert.deepEqual((await rpc('settings.get')).accounts.map(a=>a.id),beforeCancelled);
    assert.equal(calls.filter(c=>c.method==='accounts.create').length,0);await page.locator('#accounts-back').click();
    // Two normal accounts coexist in the compact floating panel renderer.
    const panel=await browser.newPage({viewport:{width:450,height:900}});await panel.exposeFunction('testRPC',invoke);await panel.addInitScript(browserFixture);panel.on('pageerror',e=>errors.push(String(e)));await panel.goto(`http://127.0.0.1:${server.address().port}/floating.html`);await panel.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);assert(await panel.getByText('个人 Codex',{exact:true}).count());assert(await panel.getByText('工作 Codex',{exact:true}).count());await panel.screenshot({path:path.join(out,'panel.png'),fullPage:true});
    for(const theme of ['light','dark'])for(const accent of ['indigo','blue','teal','purple']){
      await page.evaluate(({theme,accent})=>AieyesTheme.apply({theme,accent}),{theme,accent});
      const colors=await page.evaluate(()=>{const css=getComputedStyle(document.documentElement);return {accent:css.getPropertyValue('--accent').trim(),balance:css.getPropertyValue('--balance-blue').trim()};});assert.equal(colors.balance,colors.accent);
    }
    assert.deepEqual(errors,[]);console.log('Agents/account UI: real-core immediate save, machines, directory add/remove, retained failed drafts, independent intervals, retry, multi-machine account, auth copy/auto-connect, archive and multi-account panel passed');
  } finally {if(browser)await browser.close();server.close();core.stdin.end();fs.rmSync(root,{recursive:true,force:true});}
})().catch(error=>{console.error(error);core.kill();process.exitCode=1;});
