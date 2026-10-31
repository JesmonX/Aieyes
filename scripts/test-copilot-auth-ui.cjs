/* Copilot "GitHub 授权" with the real core and a disposable HOME. Without an Aieyes OAuth App client id,
   the dialog guides `copilot login` / `gh auth login`. Offline environment tokens and cached file usernames
   must stay unassigned. An explicit RPC fixture covers the UI consuming an already verified numeric GitHub
   identity, including retry; real identity verification and device flow have Rust/mock-HTTP coverage. */
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),os=require('node:os'),http=require('node:http'),readline=require('node:readline');
const {spawn}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright'),{serveFont,revealControl}=require('./ui-test-helpers.cjs');
const project=path.resolve(__dirname,'..'),root=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-copilot-auth-')));
const envSecret='gho_COPILOT_ENV_FIXTURE_SECRET';
const env={...process.env,AIEYES_AUTO_SCAN:'off',HOME:root,USERPROFILE:root,XDG_CONFIG_HOME:path.join(root,'.config'),GH_CONFIG_DIR:path.join(root,'.config/gh'),COPILOT_HOME:path.join(root,'.copilot'),CODEX_HOME:path.join(root,'.codex'),HTTPS_PROXY:'http://127.0.0.1:9',HTTP_PROXY:'http://127.0.0.1:9'};
for(const name of ['COPILOT_GITHUB_TOKEN','GH_TOKEN','GITHUB_TOKEN','AIEYES_GITHUB_CLIENT_ID'])delete env[name];
env.COPILOT_GITHUB_TOKEN=envSecret;
const core=spawn(path.join(project,'target/debug/aieyes-core'),['--data-dir',path.join(root,'data')],{stdio:['pipe','pipe','pipe'],env});
const pending=new Map(),calls=[],secret='gho_COPILOT_UI_FIXTURE_SECRET';let sequence=0,stderr='';
const fixtureSource='verified-numeric-identity-rpc-fixture',retryError='fixture: GitHub detection temporarily unavailable';let verifiedFixture=null;
core.stderr.on('data',b=>stderr+=b);
readline.createInterface({input:core.stdout}).on('line',line=>{const r=JSON.parse(line),p=pending.get(r.id);if(!p)return;pending.delete(r.id);r.error?p.reject(new Error(r.error.message)):p.resolve(r.result);});
core.on('exit',()=>{for(const p of pending.values())p.reject(new Error(stderr||'Core exited'));pending.clear();});
function rpc(method,params={}){return new Promise((resolve,reject)=>{const id=++sequence;pending.set(id,{resolve,reject});core.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');});}
async function invoke(method,params={}){
  if(['quotas.refresh','hosts.sample','wakeups.list','accounts.status.refresh','accounts.status.refreshBatch','accounts.status.get','accounts.devices','sources.scan','prices.list'].includes(method))return [];
  if(method==='sessions.list')return {sessions:[],sources:[],warnings:[]};
  if(method.startsWith('network.'))return {status:'ok',sites:[],testedAt:Date.now()/1000};
  if(verifiedFixture&&['copilotAuth.status','settings.get'].includes(method)){
    if(method==='copilotAuth.status'){
      verifiedFixture.statusCalls++;
      if(verifiedFixture.failNextStatus){verifiedFixture.failNextStatus=false;calls.push({method,source:fixtureSource,error:retryError});throw new Error(retryError);}
    }else verifiedFixture.settingsReads++;
    const result=structuredClone(method==='settings.get'?verifiedFixture.settings:verifiedFixture.status);
    calls.push({method,source:fixtureSource,result:JSON.stringify(result)});return result;
  }
  try{const result=await rpc(method,params);calls.push({method,source:'real-core',result:JSON.stringify(result)});return result;}
  catch(error){calls.push({method,source:'real-core',error:String(error)});throw error;}
}
function unassignedCopilot(settings,paths){
  const rows=settings.nativeConnections.filter(r=>r.provider==='copilot');
  assert.deepEqual(rows.map(r=>r.configPath).sort(),[...paths].sort(),'Every fixture credential must actually be scanned');
  for(const row of rows){
    assert.equal(row.machineId,'local');assert.equal(row.hasCredential,true);assert.equal(row.status,'observed');
    assert.deepEqual([row.accountKey??null,row.identityKey??null,row.upstreamIdentity??null],[null,null,null],'An unverified token cannot create a numeric identity or account');
  }
  assert.deepEqual(settings.accounts.filter(a=>a.provider==='copilot'),[],'Cached names cannot create Copilot accounts');
  return rows;
}
function bridge(){window.setInterval=()=>0;window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){if(command==='engine_call')return window.testRPC(args.method,args.params??{});if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'floating',effectiveMode:'floating',sessions:[],panelOpen:true};return {};}}};}
(async()=>{
  let browser,server;
  try{
    const s=await rpc('settings.get');for(const agent of s.agents){agent.enabled=agent.provider==='copilot';agent.machineIds=['local'];}
    s.appearance={...(s.appearance??{}),settingsAgentIds:['copilot']};s.proxy={mode:'custom',url:'http://127.0.0.1:9'};await rpc('settings.save',s);
    const idle=await rpc('copilotAuth.status');assert.equal(idle.phase,'idle');assert.equal(idle.deviceFlow,false,'No Aieyes OAuth App client id in this build');
    const web=path.join(project,'apps/desktop/web'),output=path.join(project,'.local/copilot-auth');fs.mkdirSync(output,{recursive:true});
    server=http.createServer((req,res)=>{if(serveFont(req,res))return;const file=path.join(web,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
    await new Promise(r=>server.listen(0,'127.0.0.1',r));browser=await chromium.launch({headless:true});
    const page=await browser.newPage({viewport:{width:1120,height:900}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.exposeFunction('testRPC',invoke);await page.addInitScript(bridge);
    await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
    await page.evaluate(()=>{state.page='settings';state.settingsTab='sources';state.settingsAgent='copilot';render();});
    await revealControl(page,'[data-copilot-auth]');const button=page.locator('[data-copilot-auth]');await button.waitFor();
    assert.equal(await button.isDisabled(),false);assert.equal(await button.innerText(),'GitHub 登录说明','No OAuth authorization button without an Aieyes OAuth App');await button.click();
    const dialog=page.locator('#copilot-auth');await dialog.waitFor();
    await dialog.locator('#copilot-auth-commands').waitFor({state:'visible'});
    assert.match(await dialog.innerText(),/copilot login/);assert.match(await dialog.innerText(),/gh auth login/);
    assert.equal(await dialog.locator('#copilot-auth-code').isVisible(),false,'No device code without Aieyes’ own OAuth App');
    assert.equal(await dialog.locator('#copilot-auth-detect').isVisible(),true);
    await page.screenshot({animations:'disabled',path:path.join(output,'external.png')});
    // Real core: the environment token cannot be verified offline, so guidance stays visible.
    await dialog.locator('#copilot-auth-detect').click();await page.waitForFunction(()=>!document.querySelector('#copilot-auth-detect').disabled);
    assert.equal(await dialog.locator('#copilot-auth-commands').isVisible(),true);
    unassignedCopilot(await rpc('settings.get'),['env:COPILOT_GITHUB_TOKEN']);
    // A cached username and someone else's app id are not a successful GitHub /user.id response.
    const appsFile=path.join(root,'.config/github-copilot/apps.json');
    fs.mkdirSync(path.dirname(appsFile),{recursive:true});
    fs.writeFileSync(appsFile,JSON.stringify({'github.com:Iv1.fixture':{user:'octo-fixture',oauth_token:secret,githubAppId:'Iv1.fixture'}}));
    await dialog.locator('#copilot-auth-detect').click();await page.waitForFunction(()=>!document.querySelector('#copilot-auth-detect').disabled);
    assert.equal(await dialog.locator('#copilot-auth-commands').isVisible(),true);
    const rejectedSettings=await rpc('settings.get');
    const rejectedRows=unassignedCopilot(rejectedSettings,['env:COPILOT_GITHUB_TOKEN',appsFile]);
    const cachedRow=rejectedRows.find(r=>r.configPath===appsFile);assert.equal(cachedRow.email,'octo-fixture','Cached display metadata was read without assigning an identity');
    const noBorrowedId=await rpc('copilotAuth.start');
    assert.equal(noBorrowedId.phase,'external');assert.equal(noBorrowedId.deviceFlow,false,'A native app id must not enable Aieyes device flow');assert.equal(noBorrowedId.userCode??null,null);
    assert(calls.filter(c=>c.source==='real-core'&&c.method==='copilotAuth.status'&&c.result).some(c=>JSON.parse(c.result).phase==='external'));
    console.log('Copilot auth real core: external guidance, unverified env/file identities kept unassigned, no borrowed client id passed');

    // Positive UI coverage only: model the secret-free settings returned after GitHub verified numeric id 71.
    // No fixture data is saved to the real core; Rust/mock-HTTP tests cover local and SSH identity verification.
    const account={id:'verified-fixture',name:'octo-fixture',provider:'copilot',identityKey:'copilot:71',upstreamIdentity:'github:71',quotaEnabled:true,connections:[],archived:false};
    const verifiedSettings=structuredClone(rejectedSettings);verifiedSettings.accounts.push(account);
    Object.assign(verifiedSettings.nativeConnections.find(r=>r.id===cachedRow.id),{accountKey:'copilot:verified-fixture',identityKey:account.identityKey,upstreamIdentity:account.upstreamIdentity,note:'已识别 GitHub 登录'});
    verifiedFixture={settings:verifiedSettings,status:{phase:'detected',message:'已识别 GitHub 登录',deviceFlow:false,userCode:null,verificationUri:null,expiresAt:null,login:'octo-fixture',commands:[]},failNextStatus:true,statusCalls:0,settingsReads:0};
    await dialog.locator('#copilot-auth-detect').click();
    await page.waitForFunction(message=>document.querySelector('#copilot-auth-error').textContent.includes(message),retryError);
    assert.equal(await dialog.locator('#copilot-auth-detect').isDisabled(),false,'A failed request must leave retry enabled');
    assert.equal(await dialog.locator('#copilot-auth-commands').isVisible(),true);assert.equal(verifiedFixture.settingsReads,0,'A failed detection must not adopt settings');
    await dialog.locator('#copilot-auth-detect').click();
    await page.waitForFunction(()=>document.querySelector('#copilot-auth-commands').hidden);
    assert.match(await dialog.locator('#copilot-auth-status').innerText(),/已识别 GitHub 登录|GitHub login detected/);
    assert.equal(await dialog.locator('#copilot-auth-error').innerText(),'','Successful retry clears the error');assert.equal(verifiedFixture.statusCalls,2);assert.equal(verifiedFixture.settingsReads,1);
    assert.deepEqual(await page.evaluate(()=>{const r=state.settings.nativeConnections.find(r=>r.accountKey==='copilot:verified-fixture');return [r?.identityKey,r?.upstreamIdentity];}),['copilot:71','github:71'],'The UI reloads the verified numeric identity fixture');
    await dialog.locator('#copilot-auth-close').click();await page.waitForFunction(()=>!document.querySelector('#copilot-auth'));
    await page.waitForFunction(()=>!document.querySelector('[data-copilot-auth]'));
    unassignedCopilot(await rpc('settings.get'),['env:COPILOT_GITHUB_TOKEN',appsFile]);
    const all=JSON.stringify(calls)+await page.content();assert(!all.includes(secret)&&!all.includes(envSecret),'Tokens never reach the page');
    assert.deepEqual(errors,[]);await page.close();
    console.log('Copilot auth RPC fixture (UI only): verified numeric identity consumed, error retry, settings reload, login entry hidden, tokens kept out of the page passed');
  } finally {
    if(browser)await browser.close();if(server)await new Promise(r=>server.close(r));core.kill();await new Promise(r=>core.exitCode!==null?r():core.once('exit',r));fs.rmSync(root,{recursive:true,force:true});
  }
})().catch(e=>{console.error(e);process.exitCode=1;});
