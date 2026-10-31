/* Real core settings + browser forms; paid service requests/keyring are simulated. */
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),os=require('node:os'),http=require('node:http'),readline=require('node:readline');
const {spawn}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const {serveFont,discardEditor}=require('./ui-test-helpers.cjs');
const project=path.resolve(__dirname,'..'),root=fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-subscriptions-'));
const core=spawn(path.join(project,'target/debug/aieyes-core'),['--data-dir',root],{stdio:['pipe','pipe','pipe']});
const pending=new Map();let seq=0,stderr='',fail=true,qoderRows=null;const calls=[];
// Desktop-only Qoder: core detection reads /Applications, so the prompt state is a fixture here.
const INSTALL='curl -fsSL https://qoder.com/install | bash && ~/.local/bin/qodercli login';
let qoderCli={apps:['Qoder'],cli:false,account:false,offerInstall:true,command:INSTALL,message:'未找到 qodercli；在终端运行安装命令后重新检测'},qoderLoggedIn=false;
core.stderr.on('data',b=>stderr+=b);
readline.createInterface({input:core.stdout}).on('line',line=>{const r=JSON.parse(line),p=pending.get(r.id);if(!p)return;pending.delete(r.id);r.error?p.reject(new Error(r.error.message)):p.resolve(r.result);});
core.on('exit',()=>{for(const p of pending.values())p.reject(new Error(stderr));pending.clear();});
function rpc(method,params={}){return new Promise((resolve,reject)=>{const id=++seq;pending.set(id,{resolve,reject});core.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');});}
async function invoke(method,params){
  calls.push({method,params});
  if(qoderRows&&method==='accounts.status.get'){
    // Preserve an asynchronous status read so the fixture also covers slow CI hosts.
    await new Promise(resolve=>setTimeout(resolve,75));
    return qoderRows;
  }
  if(qoderRows&&method==='accounts.status.refreshBatch')return qoderRows.filter(r=>!params.accountKeys||params.accountKeys.includes(r.accountKey));
  if(qoderRows&&method==='accounts.status.refresh')return qoderRows.find(r=>r.accountKey===params.accountKey&&r.sourceId===params.sourceId)??{};
  if(method==='subscriptions.openLogin')return {opened:true};
  if(method==='qoderCli.status')return qoderCli;
  if(method==='accounts.autoScan'){
    // `qodercli login` done: detection binds the new CLI login to an account.
    const settings=await rpc('settings.get');
    if(qoderLoggedIn&&!settings.accounts.some(a=>a.id==='qoder-detected')){settings.accounts.push({id:'qoder-detected',provider:'qoder',name:'Qoder',quotaEnabled:true});await rpc('settings.save',settings);}
    return {results:[],addedAccounts:qoderLoggedIn?1:0,addedConnections:0};
  }
  if(method==='subscriptions.connect'){
    if(fail)throw new Error('订阅查询失败（HTTP 401）');
    const native=params.provider==='qoder'&&params.subscription.mode==='cli';
    const settings=await rpc('settings.get'),id=params.accountId||'fixture-'+params.provider+(native?'-cli':'');
    settings.accounts.push({id,provider:params.provider,name:params.name,quotaEnabled:true});
    settings.sources.push({id:'source-'+id,provider:params.provider,name:params.name,accountId:id,enabled:true,path:'',subscription:{...params.subscription,credentialRef:native?'':'opaque-fixture'}});
    await rpc('settings.save',settings);return {settings:await rpc('settings.get'),accountId:id};
  }
  if(['sources.scan','quotas.refresh','accounts.status.refresh'].includes(method))return {};
  return rpc(method,params);
}
function fixture(){window.setInterval=()=>0;window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'floating',effectiveMode:'floating',sessions:[],panelOpen:true};if(command==='engine_call')return window.testRPC(args.method,args.params??{});return {};}}};}
(async()=>{
  const settings=await rpc('settings.get');settings.sources=[];settings.accounts=[];
  const providers=['zcode','qoder','grok','opencode','kimi','deepseek'];
  // Cursor quota comes from the IDE/CLI login (api2); the session Cookie form is gone.
  for(const p of ['cursor','trae']){const a=settings.agents.find(a=>a.provider===p);a.enabled=true;a.machineIds=['local'];}
  for(const p of providers){const a=settings.agents.find(a=>a.provider===p);a.enabled=true;a.machineIds=['local'];}
  await rpc('settings.save',settings);
  const web=path.join(project,'apps/desktop/web');
  const server=http.createServer((req,res)=>{if(serveFont(req,res))return;const file=path.join(web,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
  await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
  try{
    browser=await chromium.launch({headless:true});const page=await browser.newPage({viewport:{width:1180,height:900}}),errors=[];
    page.on('pageerror',e=>errors.push(String(e)));await page.exposeFunction('testRPC',invoke);await page.addInitScript(fixture);
    await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
    assert.equal(await page.evaluate(()=>Boolean(AieyesContract.subscriptions.cursor)),false,'Cursor has no Cookie subscription form');
    await page.context().grantPermissions(['clipboard-read','clipboard-write']);
    await page.evaluate(()=>{state.page='settings';state.settingsTab='integrations';AieyesAgentSettings.navigate({provider:'qoder'});render();});
    const install=page.locator('[data-qoder-cli]');await install.waitFor();
    assert.equal(await install.textContent(),'安装 qodercli 并登录');assert.equal(await install.isDisabled(),false);
    assert.match(await page.locator('[data-qoder-cli-row]').textContent(),/已安装 Qoder；额度通过 qodercli 查询/);
    await install.click();const qoderDialog=page.locator('#qoder-cli');await qoderDialog.waitFor();
    await page.waitForFunction(command=>document.querySelector('#qoder-cli-command')?.textContent===command,INSTALL);
    assert.match(await qoderDialog.locator('#qoder-cli-status').textContent(),/未找到 qodercli/);
    assert.match(await qoderDialog.innerText(),/Personal Access Token/);assert.match(await qoderDialog.innerText(),/qoder\.com\/account\/integrations/);
    await qoderDialog.locator('#qoder-cli-copy').click();await qoderDialog.locator('#qoder-cli-copied').getByText('已复制').waitFor();
    assert.equal(await page.evaluate(()=>navigator.clipboard.readText()),INSTALL);
    fs.mkdirSync(path.join(project,'.local/subscriptions'),{recursive:true});await page.screenshot({animations:'disabled',path:path.join(project,'.local/subscriptions/qoder-cli-install.png')});
    // Installed but not signed in: detection runs for the local CLI only and keeps the prompt.
    qoderCli={...qoderCli,cli:true,message:'已找到 qodercli；在终端完成 qodercli login 后重新检测'};
    await qoderDialog.locator('#qoder-cli-detect').click();
    await page.waitForFunction(()=>document.querySelector('#qoder-cli-status').textContent.includes('已找到 qodercli'));
    assert.deepEqual(calls.filter(c=>c.method==='accounts.autoScan').map(c=>c.params.targets),[[['qoder','local']]]);
    qoderLoggedIn=true;qoderCli={...qoderCli,account:true,offerInstall:false,message:'已识别 Qoder 登录'};
    await qoderDialog.locator('#qoder-cli-detect').click();
    await page.waitForFunction(()=>document.querySelector('#qoder-cli-status').textContent==='已识别 Qoder 登录'&&!document.querySelector('#qoder-cli-detect').disabled);
    await qoderDialog.locator('#qoder-cli-close').click();await page.waitForFunction(()=>!document.querySelector('#qoder-cli'));
    await page.waitForFunction(()=>!document.querySelector('[data-qoder-cli]'));
    assert.equal(await page.locator('[data-account-edit="qoder:qoder-detected"]').count(),1);
    // The detected fixture account would skew the per-provider form checks below.
    const detected=await rpc('settings.get');detected.accounts=detected.accounts.filter(a=>a.id!=='qoder-detected');await rpc('settings.save',detected);
    await page.evaluate(async()=>{state.settings=await api('settings.get');});

    await page.evaluate(()=>{state.page='settings';state.settingsTab='integrations';AieyesAgentSettings.navigate({provider:'trae'});render();});
    assert.equal(await page.locator('[data-account-add="trae"]').count(),0,'Encrypted Trae login has no credential form');
    for(const provider of ['trae','trae-cn']){
      const before=calls.length;
      await page.locator(`[data-usage-page="${provider}"]`).click();
      await page.waitForFunction(()=>!state.busy);
      assert(calls.slice(before).some(c=>c.method==='subscriptions.openLogin'&&c.params.provider===provider&&c.params.mode===''),provider+' opens its official usage page');
      assert(!calls.slice(before).some(c=>['subscriptions.connect','accounts.login'].includes(c.method)));
    }
    for(const provider of providers){
      await page.evaluate(p=>{state.page='settings';state.settingsTab='integrations';AieyesAgentSettings.navigate({provider:p});render();},provider);
      await page.locator(`[data-account-add="${provider}"]`).click();
      if(provider==='qoder'){
        assert.equal(await page.locator('#field-mode').inputValue(),'cli');
        assert.equal(await page.locator('#field-secret').isVisible(),false);
        assert.equal(await page.locator('#field-secret').isDisabled(),true);
        await page.locator('#field-mode').selectOption('global');
        assert.equal(await page.locator('#field-nativeConfigDir').isVisible(),false);
      }
      assert.equal(await page.locator('#field-secret').getAttribute('type'),'password');
      assert.equal(await page.locator('#field-workspaceId').isVisible(),false,provider+' has no workspace in its default mode');
      if(provider==='deepseek')assert.equal(await page.locator('#field-mode').isVisible(),false,'a single supported mode needs no picker');
      if(['grok','opencode'].includes(provider)){
        const base=provider==='grok'?'subscription':'go',other=provider==='grok'?'api':'console';
        await page.locator('#field-mode').selectOption(other);
        assert.equal(await page.locator('#field-workspaceId').isVisible(),true);
        assert.equal(await page.locator('[name=useLocalLogin]').isVisible(),false);
        await page.locator('#field-mode').selectOption(base);
        assert.equal(await page.locator('#field-workspaceId').isVisible(),false);
        assert.equal(await page.locator('[name=useLocalLogin]').isVisible(),true);
      }
      const previews=path.join(project,'.local/subscriptions');fs.mkdirSync(previews,{recursive:true});await page.screenshot({animations:'disabled',path:path.join(previews,provider+'.png')});
      await page.locator('#subscription-login').click();
      for(let attempt=0;attempt<250&&!calls.some(c=>c.method==='subscriptions.openLogin'&&c.params.provider===provider);attempt++)await new Promise(r=>setTimeout(r,20));
      assert(calls.some(c=>c.method==='subscriptions.openLogin'&&c.params.provider===provider),provider+': '+JSON.stringify(calls.slice(-5).map(c=>({method:c.method,provider:c.params.provider})))+' errors='+JSON.stringify(errors));
      await page.locator('#field-name').fill(provider+' fixture');await page.locator('#field-secret').fill('private-fixture-secret');
      fail=true;await page.locator('#editor-form button[type=submit]').click();await page.locator('#editor-error:visible').waitFor();
      assert.equal(await page.locator('#field-secret').inputValue(),'private-fixture-secret');
      assert(!(await rpc('settings.get')).accounts.some(a=>a.provider===provider));
      fail=false;await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.querySelector('#editor').open).catch(async e=>{throw new Error(provider+': '+await page.locator('#editor-error').textContent()+' / '+e.message);});
      const saved=await rpc('settings.get');assert(saved.accounts.some(a=>a.provider===provider));assert(!JSON.stringify(saved).includes('private-fixture-secret'));
    }
    await page.evaluate(()=>{AieyesAgentSettings.navigate({provider:'qoder'});render();});
    await page.locator('[data-account-add="qoder"]').click();
    await page.locator('#field-nativeConfigDir').fill('/native/qoder configuration');
    await page.screenshot({animations:'disabled',path:path.join(project,'.local/subscriptions/qoder-cli.png')});
    await page.locator('#editor-form button[type=submit]').click();
    await page.waitForFunction(()=>!document.querySelector('#editor').open);
    const nativeCall=calls.filter(c=>c.method==='subscriptions.connect'&&c.params.subscription.mode==='cli').at(-1);
    assert.equal(nativeCall.params.secret,null);assert.equal(nativeCall.params.subscription.nativeConfigDir,'/native/qoder configuration');
    const nativeSource=(await rpc('settings.get')).sources.find(s=>s.subscription?.mode==='cli');assert.equal(nativeSource.subscription.credentialRef,'');
    await page.waitForFunction(()=>!AieyesAgentSettings.statusBusy);
    // Quota connections stay available to accounts, but never count as log directories.
    const connected=await rpc('settings.get'),keyA='qoder:'+'a'.repeat(64),keyB='qoder:'+'b'.repeat(64);
    connected.hosts.push(...['fedora','144'].map(id=>({id,name:id,target:id+'.invalid',enabled:true})));
    connected.agents.find(a=>a.provider==='qoder').machineIds=['local','fedora','144'];
    for(const machine of ['local','fedora','144'])connected.sources.push({id:'qoder-history-'+machine,name:'Qoder · '+machine,provider:'qoder',path:'~/.qoder',accountId:'',hostId:machine==='local'?null:machine,enabled:true});
    await rpc('settings.save',connected);
    const machineSettings=await rpc('settings.get');qoderRows=[];
    for(const account of machineSettings.accounts.filter(a=>a.provider==='qoder'))for(const source of machineSettings.sources.filter(s=>s.provider==='qoder')){
      const machineId=source.hostId||'local',current=account.id===nativeSource.accountId?machineId!=='144':null;
      qoderRows.push({accountKey:'qoder:'+account.id,sourceId:source.id,machineId,current,credential:current,authenticated:true,identityConfirmed:true,identity:{key:machineId==='144'?keyB:keyA,email:'',subscription:'personal_standard'},checkedAt:Date.now()/1000});
    }
    await page.evaluate(settings=>{AieyesAgentSettings.accept(settings);AieyesAgentSettings.navigate({provider:'qoder'});render();},machineSettings);
    // accept() starts its own refresh. Wait for that observation before requesting
    // another read: refreshStatuses() intentionally returns while already busy.
    await page.waitForFunction(()=>!AieyesAgentSettings.statusBusy);
    await page.evaluate(async()=>{await AieyesAgentSettings.refreshStatuses(false);render();});
    await page.locator('#agent-section-machines > summary').click();
    await page.waitForFunction(()=>['local','fedora','144'].every(machine=>document.querySelector(`[data-machine-current="${machine}"][data-provider="qoder"]`)?.textContent.includes(machine==='144'?'bbbbbbbbbbbb':'aaaaaaaaaaaa')));
    for(const machine of ['local','fedora','144']){
      assert.equal(await page.locator(`[data-machine-directories="qoder"][data-machine="${machine}"]`).textContent(),'目录 1');
      const text=await page.locator(`[data-machine-current="${machine}"][data-provider="qoder"]`).textContent();
      assert(text.includes(machine==='144'?'bbbbbbbbbbbb':'aaaaaaaaaaaa'),text);
      assert(!text.includes('undefined')&&!text.includes('身份不一致'),text);
    }
    await page.locator('#agent-machines').screenshot({animations:'disabled',path:path.join(project,'.local/subscriptions/qoder-machines.png')});
    const refreshStart=calls.length;
    await page.locator('[data-machine-account-refresh="qoder"]').click();
    await page.waitForFunction(()=>!AieyesAgentSettings.statusBusy);
    const refreshed=calls.slice(refreshStart);
    assert(refreshed.some(c=>c.method==='accounts.status.refreshBatch'&&c.params.accountKeys.includes('qoder:'+nativeSource.accountId))||['local','fedora','144'].every(machine=>refreshed.some(c=>c.method==='accounts.status.refresh'&&c.params.sourceId==='qoder-history-'+machine)));
    await page.locator('[data-machine-directories="qoder"][data-machine="local"]').click();
    assert.equal(await page.locator('[data-source-advanced]').count(),1);
    assert.equal(await page.locator('[data-source-remove]').count(),1);
    assert.equal(await page.locator('[data-source-remove="'+nativeSource.id+'"]').count(),0);
    await page.locator('#directories-back').click();
    await page.locator('[data-account-edit="qoder:'+nativeSource.accountId+'"]').click();
    assert((await page.locator('[data-device-summary="fedora"]').textContent()).includes('正在使用 Qoder · aaaaaaaaaaaa'));
    // Each Qoder check starts qodercli: both interval settings state the ten-minute floor.
    assert.match(await page.locator('[data-quota-floor]').textContent(),/最少每 10 分钟/);
    assert((await page.locator('[data-device-summary="144"]').textContent()).includes('身份不一致 · 当前登录 Qoder · bbbbbbbbbbbb'));
    assert((await page.locator('body').textContent()).includes('CLI 额度连接'));
    assert((await rpc('settings.get')).sources.some(s=>s.id===nativeSource.id&&s.subscription.mode==='cli'));
    await page.evaluate(()=>{state.settingsTab='general';state.generalTab='refresh';renderSettings(false);});
    assert.match(await page.locator('[data-quota-floor]').textContent(),/最少每 10 分钟/);
    await page.evaluate(()=>{state.settingsTab='integrations';renderSettings(false);});
    await page.evaluate(()=>{AieyesAgentSettings.navigate({provider:'opencode'});render();});
    await page.locator('[data-account-add="opencode"]').click();await page.locator('[name=useLocalLogin]').check();
    await discardEditor(page);assert.equal((await rpc('settings.get')).accounts.length,providers.length+1);
    assert.deepEqual(errors,[]);console.log('Subscription forms: Qoder desktop-only install prompt and re-detect, Trae edition usage links, native Qoder, three machine identities, history-only directories, retry and opaque settings passed');
  }finally{if(browser)await browser.close();server.close();core.kill();await new Promise(r=>core.once('close',r));fs.rmSync(root,{recursive:true,force:true});}
})().catch(e=>{console.error(e);core.kill();process.exitCode=1;});
